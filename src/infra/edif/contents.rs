use super::{DesignBuilder, EndpointTarget, Parser, PendingEndpoint, PendingNet};
use crate::ir::Cell;
use anyhow::Result;

impl Parser<'_> {
    pub(super) fn parse_contents(&mut self, builder: &mut DesignBuilder) -> Result<()> {
        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "instance" => self.parse_instance(builder)?,
                    "net" => self.parse_net(builder)?,
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()
    }

    fn parse_instance(&mut self, builder: &mut DesignBuilder) -> Result<()> {
        let name = self
            .parse_name_expr()?
            .ok_or_else(|| self.error("malformed instance target"))?;
        let mut type_name = "GENERIC".to_string();
        let mut properties = Vec::new();

        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "viewRef" => {
                        if let Some(parsed_type_name) = self.parse_view_ref()? {
                            type_name = parsed_type_name;
                        }
                    }
                    "property" => {
                        if let Some(property) = self.parse_property()? {
                            properties.push(property);
                        }
                    }
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()?;

        let mut cell = Cell {
            name: name.display.clone(),
            type_name,
            ..Cell::default()
        };
        for (key, value) in properties {
            cell.set_property(key, value);
        }
        builder.push_instance(name.stable_name, cell);
        Ok(())
    }

    fn parse_view_ref(&mut self) -> Result<Option<String>> {
        let _ = self.parse_name_expr()?;
        let mut type_name = None;
        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "cellRef" => type_name = self.parse_cell_ref()?,
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()?;
        Ok(type_name)
    }

    fn parse_cell_ref(&mut self) -> Result<Option<String>> {
        let type_name = self.parse_name_expr()?.map(|name| name.display);
        while !self.peek_is_rparen()? {
            self.skip_value()?;
        }
        self.expect_rparen()?;
        Ok(type_name)
    }

    fn parse_property(&mut self) -> Result<Option<(String, String)>> {
        let key = self
            .parse_name_expr()?
            .map(|name| name.display)
            .unwrap_or_default();
        let mut value = None;

        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "integer" | "string" => value = self.parse_scalar_list()?,
                    _ => self.skip_current_list()?,
                }
            } else if value.is_none() {
                value = self.parse_atom_value()?;
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()?;

        if key.is_empty() {
            Ok(None)
        } else {
            Ok(Some((key.to_ascii_lowercase(), value.unwrap_or_default())))
        }
    }

    fn parse_scalar_list(&mut self) -> Result<Option<String>> {
        let value = self.parse_atom_value()?;
        while !self.peek_is_rparen()? {
            self.skip_value()?;
        }
        self.expect_rparen()?;
        Ok(value)
    }

    fn parse_net(&mut self, builder: &mut DesignBuilder) -> Result<()> {
        let name = self
            .parse_name_expr()?
            .map(|parsed| parsed.display)
            .ok_or_else(|| self.error("malformed net"))?;
        let mut endpoints = Vec::new();

        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "joined" => endpoints.extend(self.parse_joined(builder)?),
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()?;

        builder.push_net(PendingNet { name, endpoints });
        Ok(())
    }

    fn parse_joined(&mut self, builder: &DesignBuilder) -> Result<Vec<PendingEndpoint>> {
        let mut endpoints = Vec::new();
        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "portRef" => endpoints.push(self.parse_port_ref(builder)?),
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()?;
        Ok(endpoints)
    }

    fn parse_port_ref(&mut self, builder: &DesignBuilder) -> Result<PendingEndpoint> {
        let parsed_pin = self
            .parse_name_expr()?
            .ok_or_else(|| self.error("malformed portRef"))?;
        let raw_pin = parsed_pin.display;
        let port_name = parsed_pin
            .member
            .as_ref()
            .and_then(|member| self.resolve_current_port_member(member))
            .unwrap_or_else(|| raw_pin.clone());
        let mut pin = port_name.clone();
        let mut target = EndpointTarget::Port(port_name);

        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "instanceRef" => {
                        let instance_ref = self.parse_instance_ref()?;
                        pin = parsed_pin
                            .member
                            .as_ref()
                            .and_then(|member| {
                                builder.resolve_instance_port_member(&instance_ref, member)
                            })
                            .unwrap_or_else(|| raw_pin.clone());
                        target = EndpointTarget::InstanceRef(instance_ref);
                    }
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()?;

        Ok(PendingEndpoint { pin, target })
    }

    fn parse_instance_ref(&mut self) -> Result<String> {
        let stable_name = self
            .parse_name_expr()?
            .map(|name| name.stable_name)
            .unwrap_or_default();
        while !self.peek_is_rparen()? {
            self.skip_value()?;
        }
        self.expect_rparen()?;
        Ok(stable_name)
    }
}

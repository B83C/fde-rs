mod builder;
mod contents;
mod lexer;
mod names;

use crate::ir::{Design, Port, PortDirection};
use anyhow::{Context, Result, anyhow, bail};
use builder::{DesignBuilder, EndpointTarget, PendingEndpoint, PendingNet};
use lexer::Token;
use names::{ArraySpec, ParsedMember, ParsedName, PortDecl};
use std::{collections::BTreeMap, fs, path::Path};

pub fn load_edif(path: &Path) -> Result<Design> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read EDIF file {}", path.display()))?;
    parse_source(&source)
}

fn parse_source(source: &str) -> Result<Design> {
    Parser::new(source).parse_design()
}

struct Parser<'a> {
    source: &'a str,
    cursor: usize,
    peeked: Option<Token>,
    current_port_arrays: BTreeMap<String, ArraySpec>,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            cursor: 0,
            peeked: None,
            current_port_arrays: BTreeMap::new(),
        }
    }

    fn parse_design(mut self) -> Result<Design> {
        self.expect_lparen()?;
        self.expect_head("edif")?;
        let top_name = self
            .parse_name_expr()?
            .map(|name| name.display)
            .unwrap_or_else(|| "design".to_string());
        let mut builder = DesignBuilder::new(top_name);

        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "library" => self.parse_library(&mut builder)?,
                    "external" => self.parse_library(&mut builder)?,
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()?;

        if self.next_token()?.is_some() {
            bail!("unexpected trailing EDIF input at byte {}", self.cursor);
        }

        let top_name = builder.top_name.clone();
        let design = builder.finish();
        if design.ports.is_empty() && design.cells.is_empty() && design.nets.is_empty() {
            return Err(anyhow!(
                "missing DESIGN library top cell '{}' in EDIF",
                top_name
            ));
        }
        Ok(design)
    }

    fn parse_library(&mut self, builder: &mut DesignBuilder) -> Result<()> {
        let library_name = self
            .parse_name_expr()?
            .map(|name| name.display)
            .ok_or_else(|| self.error("missing library name"))?;
        let is_design_library = library_name == "DESIGN";

        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "cell" if is_design_library => self.parse_cell(builder)?,
                    "cell" => self.parse_library_cell(builder)?,
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()
    }

    fn parse_cell(&mut self, builder: &mut DesignBuilder) -> Result<()> {
        let cell_name = self
            .parse_name_expr()?
            .map(|name| name.display)
            .ok_or_else(|| self.error("missing cell name"))?;
        let keep = cell_name == builder.top_name;

        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "view" if keep => self.parse_view(builder)?,
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()
    }

    fn parse_library_cell(&mut self, builder: &mut DesignBuilder) -> Result<()> {
        if let Some(name) = self.parse_name_expr()? {
            builder.register_library_cell(name.clone());
            while !self.peek_is_rparen()? {
                if self.peek_is_lparen()? {
                    self.expect_lparen()?;
                    let head = self.expect_atom()?;
                    match head.as_str() {
                        "view" => self.parse_library_cell_view(builder, &name.display)?,
                        _ => self.skip_current_list()?,
                    }
                } else {
                    self.skip_value()?;
                }
            }
            return self.expect_rparen();
        }
        while !self.peek_is_rparen()? {
            self.skip_value()?;
        }
        self.expect_rparen()
    }

    fn parse_library_cell_view(
        &mut self,
        builder: &mut DesignBuilder,
        cell_name: &str,
    ) -> Result<()> {
        let saved_port_arrays = std::mem::take(&mut self.current_port_arrays);
        let _ = self.parse_name_expr()?;
        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "interface" => self.parse_library_cell_interface(builder, cell_name)?,
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()?;
        self.current_port_arrays = saved_port_arrays;
        Ok(())
    }

    fn parse_library_cell_interface(
        &mut self,
        builder: &mut DesignBuilder,
        cell_name: &str,
    ) -> Result<()> {
        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "port" => self.parse_library_cell_port(builder, cell_name)?,
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()
    }

    fn parse_library_cell_port(
        &mut self,
        builder: &mut DesignBuilder,
        cell_name: &str,
    ) -> Result<()> {
        let decl = self.parse_port_decl_names()?;
        register_library_cell_port_decl(builder, cell_name, &decl);

        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "direction" => {
                        let _ = self.parse_direction()?;
                    }
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()
    }

    fn parse_view(&mut self, builder: &mut DesignBuilder) -> Result<()> {
        self.current_port_arrays.clear();
        let _ = self.parse_name_expr()?;
        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "interface" => self.parse_interface(builder)?,
                    "contents" => self.parse_contents(builder)?,
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()
    }

    fn parse_interface(&mut self, builder: &mut DesignBuilder) -> Result<()> {
        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "port" => self.parse_port(builder)?,
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()
    }

    fn parse_port(&mut self, builder: &mut DesignBuilder) -> Result<()> {
        let decl = self.parse_port_decl_names()?;
        if decl.names.is_empty() {
            return Err(self.error("malformed port"));
        }
        let mut direction = PortDirection::Input;

        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "direction" => direction = self.parse_direction()?,
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()?;

        if let (Some(array_key), Some(array_spec)) = (decl.array_key, decl.array_spec) {
            self.current_port_arrays.insert(array_key, array_spec);
        }

        for name in decl.names {
            let mut port = Port::new(name, direction.clone());
            port.width = 1;
            builder.push_port(port);
        }
        Ok(())
    }

    fn parse_direction(&mut self) -> Result<PortDirection> {
        let value = self
            .parse_name_expr()?
            .map(|name| name.display)
            .unwrap_or_else(|| "INPUT".to_string());
        while !self.peek_is_rparen()? {
            self.skip_value()?;
        }
        self.expect_rparen()?;
        Ok(value.parse().unwrap_or(PortDirection::Input))
    }
}

fn register_library_cell_port_decl(builder: &mut DesignBuilder, cell_name: &str, decl: &PortDecl) {
    if let (Some(array_key), Some(array_spec)) = (&decl.array_key, &decl.array_spec) {
        builder.register_library_cell_port_array(cell_name, array_key, array_spec.clone());
    }
}

#[cfg(test)]
mod tests;

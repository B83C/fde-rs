use crate::{
    domain::{CellKind, PinRole, PrimitiveKind},
    ir::{Cell, CellPin, Design, Endpoint, EndpointKind, Net, Port, PortDirection},
};
use anyhow::{Context, Result, anyhow, bail};
use std::{collections::BTreeMap, fs, path::Path};

pub fn load_edif(path: &Path) -> Result<Design> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read EDIF file {}", path.display()))?;
    parse_source(&source)
}

fn parse_source(source: &str) -> Result<Design> {
    Parser::new(source).parse_design()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    LParen,
    RParen,
    Atom(String),
}

#[derive(Debug, Clone)]
struct ParsedName {
    display: String,
    stable_name: String,
    member: Option<ParsedMember>,
}

#[derive(Debug, Clone)]
struct ParsedMember {
    base_key: String,
    ordinal: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArrayRange {
    msb: usize,
    lsb: usize,
}

impl ArrayRange {
    fn from_width(width: usize) -> Self {
        Self {
            msb: width.saturating_sub(1),
            lsb: 0,
        }
    }

    fn width(self) -> usize {
        self.msb.abs_diff(self.lsb).saturating_add(1)
    }

    fn member_name(self, base: &str, ordinal: usize) -> Option<String> {
        if ordinal >= self.width() {
            return None;
        }
        let index = if self.msb >= self.lsb {
            self.msb - ordinal
        } else {
            self.msb + ordinal
        };
        Some(indexed_name(base, index))
    }

    fn declaration_names(self, base: &str) -> Vec<String> {
        (0..self.width())
            .filter_map(|ordinal| self.member_name(base, ordinal))
            .collect()
    }
}

#[derive(Debug, Clone)]
struct ArraySpec {
    display_base: String,
    range: ArrayRange,
}

#[derive(Debug, Clone)]
struct PortDecl {
    names: Vec<String>,
    array_key: Option<String>,
    array_spec: Option<ArraySpec>,
}

#[derive(Debug, Clone)]
struct PendingEndpoint {
    pin: String,
    target: EndpointTarget,
}

#[derive(Debug, Clone)]
enum EndpointTarget {
    Port(String),
    InstanceRef(String),
}

#[derive(Debug, Clone)]
struct PendingNet {
    name: String,
    endpoints: Vec<PendingEndpoint>,
}

#[derive(Debug, Clone)]
struct DesignBuilder {
    top_name: String,
    design: Design,
    cell_types: BTreeMap<String, String>,
    library_cell_names: BTreeMap<String, String>,
    instance_names: BTreeMap<String, String>,
    pending_nets: Vec<PendingNet>,
}

impl DesignBuilder {
    fn new(top_name: String) -> Self {
        let mut design = Design {
            name: top_name.clone(),
            stage: "mapped".to_string(),
            ..Design::default()
        };
        design.metadata.source_format = "edif".to_string();
        Self {
            top_name,
            design,
            cell_types: BTreeMap::new(),
            library_cell_names: BTreeMap::new(),
            instance_names: BTreeMap::new(),
            pending_nets: Vec::new(),
        }
    }

    fn push_port(&mut self, port: Port) {
        self.design.ports.push(port);
    }

    fn push_instance(&mut self, instance_ref: String, mut cell: Cell) {
        if let Some(resolved_type_name) = self.library_cell_names.get(&cell.type_name) {
            cell.type_name = resolved_type_name.clone();
        }
        cell.kind = classify_cell_kind(&cell.type_name);
        self.instance_names.insert(instance_ref, cell.name.clone());
        self.cell_types
            .insert(cell.name.clone(), cell.type_name.clone());
        self.design.cells.push(cell);
    }

    fn register_library_cell(&mut self, name: ParsedName) {
        self.library_cell_names
            .insert(name.stable_name, name.display);
    }

    fn push_net(&mut self, net: PendingNet) {
        self.pending_nets.push(net);
    }

    fn finish(mut self) -> Design {
        for pending in self.pending_nets.drain(..) {
            let endpoints = pending
                .endpoints
                .into_iter()
                .map(|endpoint| match endpoint.target {
                    EndpointTarget::Port(name) => Endpoint {
                        kind: EndpointKind::Port,
                        name,
                        pin: endpoint.pin,
                    },
                    EndpointTarget::InstanceRef(instance_ref) => Endpoint {
                        kind: EndpointKind::Cell,
                        name: self
                            .instance_names
                            .get(&instance_ref)
                            .cloned()
                            .unwrap_or(instance_ref),
                        pin: endpoint.pin,
                    },
                })
                .collect::<Vec<_>>();
            let (driver, sinks) = split_endpoints(&self.design, &self.cell_types, &endpoints);
            self.design.nets.push(Net {
                name: pending.name,
                driver,
                sinks,
                ..Net::default()
            });
        }

        for cell in &mut self.design.cells {
            cell.inputs.clear();
            cell.outputs.clear();
        }
        let cell_index = self
            .design
            .cells
            .iter()
            .enumerate()
            .map(|(index, cell)| (cell.name.clone(), index))
            .collect::<BTreeMap<_, _>>();
        for net in &self.design.nets {
            if let Some(driver) = &net.driver
                && driver.is_cell()
                && let Some(index) = cell_index.get(&driver.name)
            {
                self.design.cells[*index]
                    .outputs
                    .push(CellPin::new(driver.pin.clone(), net.name.clone()));
            }
            for sink in &net.sinks {
                if sink.is_cell()
                    && let Some(index) = cell_index.get(&sink.name)
                {
                    self.design.cells[*index]
                        .inputs
                        .push(CellPin::new(sink.pin.clone(), net.name.clone()));
                }
            }
        }

        self.design
    }
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
            builder.register_library_cell(name);
        }
        while !self.peek_is_rparen()? {
            self.skip_value()?;
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

    fn parse_contents(&mut self, builder: &mut DesignBuilder) -> Result<()> {
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
                    "joined" => endpoints.extend(self.parse_joined()?),
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

    fn parse_joined(&mut self) -> Result<Vec<PendingEndpoint>> {
        let mut endpoints = Vec::new();
        while !self.peek_is_rparen()? {
            if self.peek_is_lparen()? {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                match head.as_str() {
                    "portRef" => endpoints.push(self.parse_port_ref()?),
                    _ => self.skip_current_list()?,
                }
            } else {
                self.skip_value()?;
            }
        }
        self.expect_rparen()?;
        Ok(endpoints)
    }

    fn parse_port_ref(&mut self) -> Result<PendingEndpoint> {
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
                        target = EndpointTarget::InstanceRef(self.parse_instance_ref()?);
                        pin = raw_pin.clone();
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

    fn parse_name_expr(&mut self) -> Result<Option<ParsedName>> {
        match self.peek_token()? {
            Some(Token::Atom(_)) => Ok(self.parse_atom_value()?.map(|value| ParsedName {
                display: value.clone(),
                stable_name: value,
                member: None,
            })),
            Some(Token::LParen) => {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                let parsed = match head.as_str() {
                    "rename" => {
                        let stable_name = self
                            .parse_name_expr()?
                            .map(|name| name.display)
                            .unwrap_or_default();
                        let display = self
                            .parse_name_expr()?
                            .map(|name| name.display)
                            .unwrap_or_else(|| stable_name.clone());
                        while !self.peek_is_rparen()? {
                            self.skip_value()?;
                        }
                        self.expect_rparen()?;
                        Some(ParsedName {
                            display,
                            stable_name,
                            member: None,
                        })
                    }
                    "array" => {
                        let value = self
                            .parse_name_expr()?
                            .map(|name| name.display)
                            .unwrap_or_default();
                        while !self.peek_is_rparen()? {
                            self.skip_value()?;
                        }
                        self.expect_rparen()?;
                        Some(ParsedName {
                            display: value.clone(),
                            stable_name: value,
                            member: None,
                        })
                    }
                    "member" => {
                        let value = self.parse_name_expr()?.unwrap_or(ParsedName {
                            display: String::new(),
                            stable_name: String::new(),
                            member: None,
                        });
                        let index = self
                            .parse_atom_value()?
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(0);
                        while !self.peek_is_rparen()? {
                            self.skip_value()?;
                        }
                        self.expect_rparen()?;
                        let indexed = indexed_name(&value.display, index);
                        Some(ParsedName {
                            display: indexed.clone(),
                            stable_name: indexed,
                            member: Some(ParsedMember {
                                base_key: value.stable_name,
                                ordinal: index,
                            }),
                        })
                    }
                    _ => {
                        self.skip_current_list()?;
                        None
                    }
                };
                Ok(parsed)
            }
            Some(Token::RParen) | None => Ok(None),
        }
    }

    fn parse_port_decl_names(&mut self) -> Result<PortDecl> {
        match self.peek_token()? {
            Some(Token::Atom(_)) => Ok(PortDecl {
                names: self
                    .parse_atom_value()?
                    .map(|name| vec![name])
                    .unwrap_or_default(),
                array_key: None,
                array_spec: None,
            }),
            Some(Token::LParen) => {
                self.expect_lparen()?;
                let head = self.expect_atom()?;
                let decl = match head.as_str() {
                    "array" => {
                        let base = self.parse_name_expr()?.unwrap_or(ParsedName {
                            display: String::new(),
                            stable_name: String::new(),
                            member: None,
                        });
                        let width = self
                            .parse_atom_value()?
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(1);
                        while !self.peek_is_rparen()? {
                            self.skip_value()?;
                        }
                        self.expect_rparen()?;
                        let (display_base, range) = match parse_bus_range_name(&base.display) {
                            Some((display_base, range)) => {
                                if range.width() != width {
                                    return Err(self.error(format!(
                                        "array size mismatch for '{}' (declared {width}, range width {})",
                                        base.display,
                                        range.width()
                                    )));
                                }
                                (display_base, range)
                            }
                            None => (base.display.clone(), ArrayRange::from_width(width)),
                        };
                        PortDecl {
                            names: range.declaration_names(&display_base),
                            array_key: Some(base.stable_name),
                            array_spec: Some(ArraySpec {
                                display_base,
                                range,
                            }),
                        }
                    }
                    "rename" => {
                        let _ = self.parse_name_expr()?;
                        let display = self
                            .parse_name_expr()?
                            .map(|name| name.display)
                            .unwrap_or_default();
                        while !self.peek_is_rparen()? {
                            self.skip_value()?;
                        }
                        self.expect_rparen()?;
                        PortDecl {
                            names: vec![display],
                            array_key: None,
                            array_spec: None,
                        }
                    }
                    _ => {
                        self.skip_current_list()?;
                        PortDecl {
                            names: Vec::new(),
                            array_key: None,
                            array_spec: None,
                        }
                    }
                };
                Ok(decl)
            }
            Some(Token::RParen) | None => Ok(PortDecl {
                names: Vec::new(),
                array_key: None,
                array_spec: None,
            }),
        }
    }

    fn resolve_current_port_member(&self, member: &ParsedMember) -> Option<String> {
        self.current_port_arrays
            .get(&member.base_key)
            .and_then(|spec| spec.range.member_name(&spec.display_base, member.ordinal))
    }

    fn parse_atom_value(&mut self) -> Result<Option<String>> {
        match self.next_token()? {
            Some(Token::Atom(value)) => Ok(Some(value)),
            Some(Token::LParen) => {
                self.skip_open_list()?;
                Ok(None)
            }
            Some(Token::RParen) | None => Ok(None),
        }
    }

    fn expect_head(&mut self, expected: &str) -> Result<()> {
        let head = self.expect_atom()?;
        if head == expected {
            Ok(())
        } else {
            Err(self.error(format!("expected head '{expected}', found '{head}'")))
        }
    }

    fn expect_lparen(&mut self) -> Result<()> {
        match self.next_token()? {
            Some(Token::LParen) => Ok(()),
            Some(Token::Atom(value)) => Err(self.error(format!("expected '(', found '{value}'"))),
            Some(Token::RParen) => Err(self.error("expected '(', found ')'")),
            None => Err(self.error("unexpected end of EDIF input")),
        }
    }

    fn expect_rparen(&mut self) -> Result<()> {
        match self.next_token()? {
            Some(Token::RParen) => Ok(()),
            Some(Token::Atom(value)) => Err(self.error(format!("expected ')', found '{value}'"))),
            Some(Token::LParen) => Err(self.error("expected ')', found '('")),
            None => Err(self.error("unexpected end of EDIF input")),
        }
    }

    fn expect_atom(&mut self) -> Result<String> {
        match self.next_token()? {
            Some(Token::Atom(value)) => Ok(value),
            Some(Token::LParen) => Err(self.error("expected atom, found '('")),
            Some(Token::RParen) => Err(self.error("expected atom, found ')'")),
            None => Err(self.error("unexpected end of EDIF input")),
        }
    }

    fn peek_is_lparen(&mut self) -> Result<bool> {
        Ok(matches!(self.peek_token()?, Some(Token::LParen)))
    }

    fn peek_is_rparen(&mut self) -> Result<bool> {
        Ok(matches!(self.peek_token()?, Some(Token::RParen)))
    }

    fn skip_value(&mut self) -> Result<()> {
        match self.next_token()? {
            Some(Token::LParen) => self.skip_open_list(),
            Some(Token::Atom(_)) => Ok(()),
            Some(Token::RParen) => Err(self.error("unexpected ')' in EDIF input")),
            None => Err(self.error("unexpected end of EDIF input")),
        }
    }

    fn skip_current_list(&mut self) -> Result<()> {
        let mut depth = 1usize;
        while depth > 0 {
            match self.next_token()? {
                Some(Token::LParen) => depth += 1,
                Some(Token::RParen) => depth = depth.saturating_sub(1),
                Some(Token::Atom(_)) => {}
                None => return Err(self.error("unterminated EDIF list")),
            }
        }
        Ok(())
    }

    fn skip_open_list(&mut self) -> Result<()> {
        let mut depth = 1usize;
        while depth > 0 {
            match self.next_token()? {
                Some(Token::LParen) => depth += 1,
                Some(Token::RParen) => depth = depth.saturating_sub(1),
                Some(Token::Atom(_)) => {}
                None => return Err(self.error("unterminated EDIF list")),
            }
        }
        Ok(())
    }

    fn peek_token(&mut self) -> Result<Option<Token>> {
        if self.peeked.is_none() {
            self.peeked = self.read_token()?;
        }
        Ok(self.peeked.clone())
    }

    fn next_token(&mut self) -> Result<Option<Token>> {
        if let Some(token) = self.peeked.take() {
            return Ok(Some(token));
        }
        self.read_token()
    }

    fn read_token(&mut self) -> Result<Option<Token>> {
        self.skip_whitespace_and_comments();
        let Some(ch) = self.peek_char() else {
            return Ok(None);
        };
        let token = match ch {
            '(' => {
                self.bump_char();
                Token::LParen
            }
            ')' => {
                self.bump_char();
                Token::RParen
            }
            '"' => Token::Atom(self.read_quoted_string()?),
            _ => Token::Atom(self.read_atom()),
        };
        Ok(Some(token))
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            while self.peek_char().is_some_and(char::is_whitespace) {
                self.bump_char();
            }
            if self.peek_char() == Some(';') {
                while let Some(ch) = self.bump_char() {
                    if ch == '\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn read_quoted_string(&mut self) -> Result<String> {
        match self.bump_char() {
            Some('"') => {}
            _ => return Err(self.error("expected string literal")),
        }
        let mut value = String::new();
        while let Some(ch) = self.bump_char() {
            match ch {
                '"' => return Ok(value),
                '\\' => {
                    let escaped = self
                        .bump_char()
                        .ok_or_else(|| self.error("unterminated escape sequence"))?;
                    value.push(escaped);
                }
                other => value.push(other),
            }
        }
        Err(self.error("unterminated string literal"))
    }

    fn read_atom(&mut self) -> String {
        let start = self.cursor;
        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace() || matches!(ch, '(' | ')') {
                break;
            }
            self.bump_char();
        }
        self.source[start..self.cursor].to_string()
    }

    fn peek_char(&self) -> Option<char> {
        self.source[self.cursor..].chars().next()
    }

    fn bump_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.cursor += ch.len_utf8();
        Some(ch)
    }

    fn error(&self, message: impl Into<String>) -> anyhow::Error {
        anyhow!("{} at byte {}", message.into(), self.cursor)
    }
}

fn split_endpoints(
    design: &Design,
    cell_types: &BTreeMap<String, String>,
    endpoints: &[Endpoint],
) -> (Option<Endpoint>, Vec<Endpoint>) {
    let port_dirs = design
        .ports
        .iter()
        .map(|port| (port.name.clone(), port.direction.clone()))
        .collect::<BTreeMap<_, _>>();

    let mut sources = Vec::new();
    let mut sinks = Vec::new();
    for endpoint in endpoints {
        let is_source = match endpoint.kind {
            EndpointKind::Port => port_dirs
                .get(&endpoint.name)
                .map(PortDirection::is_input_like)
                .unwrap_or(false),
            EndpointKind::Cell => cell_types
                .get(&endpoint.name)
                .map(|type_name| is_output_pin(type_name, &endpoint.pin))
                .unwrap_or(false),
            EndpointKind::Unknown => false,
        };
        if is_source {
            sources.push(endpoint.clone());
        } else {
            sinks.push(endpoint.clone());
        }
    }

    let driver = sources
        .iter()
        .find(|endpoint| endpoint.is_cell())
        .cloned()
        .or_else(|| sources.first().cloned())
        .or_else(|| sinks.first().cloned());
    let sinks = endpoints
        .iter()
        .filter(|endpoint| Some(endpoint.key()) != driver.as_ref().map(Endpoint::key))
        .cloned()
        .collect::<Vec<_>>();
    (driver, sinks)
}

fn classify_cell_kind(type_name: &str) -> CellKind {
    match PrimitiveKind::classify("", type_name) {
        PrimitiveKind::Lut { .. } => CellKind::Lut,
        PrimitiveKind::FlipFlop => CellKind::Ff,
        PrimitiveKind::Latch => CellKind::Latch,
        PrimitiveKind::Constant(_) => CellKind::Constant,
        PrimitiveKind::Buffer => CellKind::Buffer,
        PrimitiveKind::Io => CellKind::Io,
        PrimitiveKind::GlobalClockBuffer => CellKind::GlobalClockBuffer,
        PrimitiveKind::Generic | PrimitiveKind::Unknown => CellKind::Generic,
    }
}

fn is_output_pin(type_name: &str, pin: &str) -> bool {
    PinRole::classify_output_pin(PrimitiveKind::classify("", type_name), pin).is_output_like()
}

fn indexed_name(base: &str, index: usize) -> String {
    format!("{base}[{index}]")
}

fn parse_bus_range_name(name: &str) -> Option<(String, ArrayRange)> {
    let (open, close) = [('[' , ']'), ('(', ')'), ('<', '>')]
        .into_iter()
        .find(|(open, close)| name.ends_with(*close) && name.contains(*open))?;
    let split = name.rfind(open)?;
    let base = name[..split].to_string();
    let range = &name[split + 1..name.len().checked_sub(close.len_utf8())?];
    let (msb, lsb) = range.split_once(':')?;
    Some((
        base,
        ArrayRange {
            msb: msb.parse().ok()?,
            lsb: lsb.parse().ok()?,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::parse_source;

    #[test]
    fn parses_renamed_instance_references() {
        let design = parse_source(
            r#"
            (edif top
              (library DESIGN
                (cell top
                  (view NETLIST
                    (interface
                      (port a (direction INPUT))
                      (port y (direction OUTPUT)))
                    (contents
                      (instance (rename id00001 u_lut)
                        (viewRef NETLIST (cellRef LUT2 (libraryRef LIB))))
                      (net net0
                        (joined
                          (portRef a)
                          (portRef ADR0 (instanceRef id00001))))
                      (net net1
                        (joined
                          (portRef O (instanceRef id00001))
                          (portRef y))))))))
            "#,
        )
        .expect("parse rename");

        let net0 = design
            .nets
            .iter()
            .find(|net| net.name == "net0")
            .expect("net0");
        let sink = net0.sinks.first().expect("sink");
        assert_eq!(sink.name, "u_lut");
        assert_eq!(sink.pin, "ADR0");
    }

    #[test]
    fn resolves_renamed_external_library_cells_before_classifying_instances() {
        let design = parse_source(
            r#"
            (edif top
              (external LIB
                (cell (rename id00001 "$_DFF_P_")
                  (cellType GENERIC)
                  (view VIEW_NETLIST
                    (viewType NETLIST)
                    (interface
                      (port C (direction INPUT))
                      (port D (direction INPUT))
                      (port Q (direction OUTPUT))))))
              (library DESIGN
                (cell top
                  (view NETLIST
                    (interface
                      (port clk (direction INPUT))
                      (port q (direction OUTPUT)))
                    (contents
                      (instance (rename id100 ff0)
                        (viewRef NETLIST (cellRef id00001 (libraryRef LIB))))
                      (net clk_net
                        (joined
                          (portRef clk)
                          (portRef C (instanceRef id100))))
                      (net q_net
                        (joined
                          (portRef Q (instanceRef id100))
                          (portRef q))))))))
            "#,
        )
        .expect("parse external rename");

        let cell = design
            .cells
            .iter()
            .find(|cell| cell.name == "ff0")
            .expect("ff0");
        assert_eq!(cell.type_name, "$_DFF_P_");
        assert_eq!(cell.kind.as_str(), "ff");
    }

    #[test]
    fn parses_string_properties_and_comments() {
        let design = parse_source(
            r#"
            (edif top
              ; comment should be ignored
              (library DESIGN
                (cell top
                  (view NETLIST
                    (interface (port a (direction INPUT)))
                    (contents
                      (instance u0
                        (viewRef NETLIST (cellRef LUT2 (libraryRef LIB)))
                        (property LABEL (string "hello world"))))))))
            "#,
        )
        .expect("parse property");

        let cell = design
            .cells
            .iter()
            .find(|cell| cell.name == "u0")
            .expect("cell");
        assert_eq!(cell.property("label"), Some("hello world"));
    }

    #[test]
    fn parses_integer_properties_on_structural_luts() {
        let design = parse_source(
            r#"
            (edif top
              (library DESIGN
                (cell top
                  (view NETLIST
                    (interface (port a (direction INPUT)))
                    (contents
                      (instance u0
                        (viewRef NETLIST (cellRef LUT2 (libraryRef LIB)))
                        (property INIT (integer 10))))))))
            "#,
        )
        .expect("parse integer property");

        let cell = design
            .cells
            .iter()
            .find(|cell| cell.name == "u0")
            .expect("cell");
        assert_eq!(cell.property("init"), Some("10"));
    }

    #[test]
    fn parses_array_ports_and_member_references() {
        let design = parse_source(
            r#"
            (edif top
              (library DESIGN
                (cell top
                  (view NETLIST
                    (interface
                      (port clk (direction INPUT))
                      (port (array bus_in 2) (direction INPUT))
                      (port (array bus_out 2) (direction OUTPUT)))
                    (contents
                      (instance u0
                        (viewRef NETLIST (cellRef LUT2 (libraryRef LIB))))
                      (net net0
                        (joined
                          (portRef (member bus_in 0))
                          (portRef ADR0 (instanceRef u0))))
                      (net net1
                        (joined
                          (portRef (member bus_in 1))
                          (portRef ADR1 (instanceRef u0))))
                      (net net2
                        (joined
                          (portRef O (instanceRef u0))
                          (portRef (member bus_out 1)))))))))
            "#,
        )
        .expect("parse array ports");

        let port_names = design
            .ports
            .iter()
            .map(|port| port.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            port_names,
            vec!["clk", "bus_in[1]", "bus_in[0]", "bus_out[1]", "bus_out[0]"]
        );

        let net0 = design
            .nets
            .iter()
            .find(|net| net.name == "net0")
            .expect("net0");
        assert_eq!(
            net0.driver.as_ref().map(|driver| driver.name.as_str()),
            Some("bus_in[1]")
        );

        let net2 = design
            .nets
            .iter()
            .find(|net| net.name == "net2")
            .expect("net2");
        assert_eq!(
            net2.sinks.first().map(|sink| sink.name.as_str()),
            Some("bus_out[0]")
        );
    }

    #[test]
    fn resolves_renamed_array_ports_using_member_ordinals() {
        let design = parse_source(
            r#"
            (edif top
              (library DESIGN
                (cell top
                  (view NETLIST
                    (interface
                      (port (array (rename BUS "bus[3:1]") 3) (direction OUTPUT)))
                    (contents
                      (instance u0
                        (viewRef NETLIST (cellRef LUT2 (libraryRef LIB))))
                      (net net0
                        (joined
                          (portRef O (instanceRef u0))
                          (portRef (member BUS 0))))
                      (net net1
                        (joined
                          (portRef O (instanceRef u0))
                          (portRef (member BUS 1))))
                      (net net2
                        (joined
                          (portRef O (instanceRef u0))
                          (portRef (member BUS 2)))))))))
            "#,
        )
        .expect("parse renamed array");

        let port_names = design
            .ports
            .iter()
            .map(|port| port.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(port_names, vec!["bus[3]", "bus[2]", "bus[1]"]);

        let net0 = design
            .nets
            .iter()
            .find(|net| net.name == "net0")
            .expect("net0");
        assert_eq!(
            net0.sinks.first().map(|sink| sink.name.as_str()),
            Some("bus[3]")
        );

        let net2 = design
            .nets
            .iter()
            .find(|net| net.name == "net2")
            .expect("net2");
        assert_eq!(
            net2.sinks.first().map(|sink| sink.name.as_str()),
            Some("bus[1]")
        );
    }
}

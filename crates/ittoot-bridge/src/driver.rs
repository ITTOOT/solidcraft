//! The one trait every PLC link implements, plus a simulated link so the model and the tables can
//! be worked on with no PLC. Real links (OPC UA, S7, Modbus) implement `PlcDriver` in their own
//! modules later; nothing above this file knows which one is in use.

use crate::connection::LinkState;
use crate::error::{Error, Result};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Real(f64),
    Text(String),
}

impl Value {
    pub fn show(&self) -> String {
        match self {
            Value::Bool(b) => b.to_string().to_uppercase(),
            Value::Int(i) => i.to_string(),
            Value::Real(r) => format!("{r}"),
            Value::Text(t) => t.clone(),
        }
    }

    /// The coarse class used to compare a PLC type with a model property type.
    pub fn class(&self) -> TypeClass {
        match self {
            Value::Bool(_) => TypeClass::Bool,
            Value::Int(_) => TypeClass::Integer,
            Value::Real(_) => TypeClass::Real,
            Value::Text(_) => TypeClass::Text,
        }
    }

    /// Read what a person typed into a table cell, as the given type.
    pub fn parse(class: TypeClass, text: &str) -> Result<Value> {
        let t = text.trim();
        let bad = || Error::Invalid(format!("'{t}' is not a valid {class:?} value"));
        match class {
            TypeClass::Bool => match t.to_ascii_lowercase().as_str() {
                "1" | "true" => Ok(Value::Bool(true)),
                "0" | "false" => Ok(Value::Bool(false)),
                _ => Err(bad()),
            },
            TypeClass::Integer => t.parse().map(Value::Int).map_err(|_| bad()),
            TypeClass::Real => t.parse::<f64>().ok().filter(|r| r.is_finite()).map(Value::Real).ok_or_else(bad),
            TypeClass::Text => Ok(Value::Text(t.to_string())),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeClass {
    Bool,
    Integer,
    Real,
    Text,
}

/// The class of an S7 type name (`Bool`, `DInt`, `LReal`, `String[24]`, `Time` ...).
pub fn class_of_plc_type(ty: &str) -> Option<TypeClass> {
    let t = ty.trim().to_ascii_lowercase();
    match t.as_str() {
        "bool" | "di" | "do" => Some(TypeClass::Bool),
        "byte" | "word" | "dword" | "lword" | "sint" | "usint" | "int" | "uint" | "dint" | "udint" | "lint" | "ulint" | "time" | "ltime" | "ai" | "ao" => Some(TypeClass::Integer),
        "real" | "lreal" => Some(TypeClass::Real),
        _ if t.starts_with("string") || t.starts_with("wstring") || t == "char" => Some(TypeClass::Text),
        _ => None,
    }
}

/// A start value as written in SCL (`TRUE`, `20.0`, `'text'`, `T#50ms`) as a value; `None` for what is not a plain literal.
pub fn value_from_literal(literal: &str) -> Option<Value> {
    let t = literal.trim();
    if t.eq_ignore_ascii_case("true") {
        return Some(Value::Bool(true));
    }
    if t.eq_ignore_ascii_case("false") {
        return Some(Value::Bool(false));
    }
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        return t.get(1..t.len() - 1).map(|s| Value::Text(s.to_string()));
    }
    let n = t.rsplit('#').next().unwrap_or(t);
    if let Ok(i) = n.parse::<i64>() {
        return Some(Value::Int(i));
    }
    n.parse::<f64>().ok().filter(|f| f.is_finite()).map(Value::Real)
}

pub trait PlcDriver {
    fn state(&self) -> LinkState;
    fn connect(&mut self) -> Result<()>;
    fn disconnect(&mut self);
    /// One answer per key, `None` for a key the PLC does not have.
    fn read(&mut self, keys: &[String]) -> Result<Vec<Option<Value>>>;
    fn write(&mut self, key: &str, value: Value) -> Result<()>;
}

/// Values held in memory: the "PLC" for working offline, and for tests.
#[derive(Debug, Default)]
pub struct SimDriver {
    values: BTreeMap<String, Value>,
    up: bool,
}

impl SimDriver {
    pub fn with_values(values: impl IntoIterator<Item = (String, Value)>) -> Self {
        SimDriver { values: values.into_iter().collect(), up: false }
    }
}

impl PlcDriver for SimDriver {
    fn state(&self) -> LinkState {
        if self.up { LinkState::Connected } else { LinkState::Disconnected }
    }

    fn connect(&mut self) -> Result<()> {
        self.up = true;
        Ok(())
    }

    fn disconnect(&mut self) {
        self.up = false;
    }

    fn read(&mut self, keys: &[String]) -> Result<Vec<Option<Value>>> {
        if !self.up {
            return Err(Error::NotConnected("simulated link".into()));
        }
        Ok(keys.iter().map(|k| self.values.get(k).cloned()).collect())
    }

    fn write(&mut self, key: &str, value: Value) -> Result<()> {
        if !self.up {
            return Err(Error::NotConnected("simulated link".into()));
        }
        self.values.insert(key.to_string(), value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_needs_connect_then_reads_and_writes() {
        let mut d = SimDriver::with_values([("DB.a".to_string(), Value::Int(3))]);
        assert!(d.read(&["DB.a".into()]).is_err());
        d.connect().unwrap();
        assert_eq!(d.read(&["DB.a".into(), "DB.b".into()]).unwrap(), [Some(Value::Int(3)), None]);
        d.write("DB.b", Value::Bool(true)).unwrap();
        assert_eq!(d.read(&["DB.b".into()]).unwrap(), [Some(Value::Bool(true))]);
        assert_eq!(d.state(), LinkState::Connected);
    }

    #[test]
    fn literals() {
        assert_eq!(value_from_literal("TRUE"), Some(Value::Bool(true)));
        assert_eq!(value_from_literal("20.0"), Some(Value::Real(20.0)));
        assert_eq!(value_from_literal("-3"), Some(Value::Int(-3)));
        assert_eq!(value_from_literal("'Station 1'"), Some(Value::Text("Station 1".into())));
        assert_eq!(value_from_literal("\"STATION_SERVICE\""), None);
    }

    #[test]
    fn types_and_typed_cells() {
        assert_eq!(class_of_plc_type("DInt"), Some(TypeClass::Integer));
        assert_eq!(class_of_plc_type("String[24]"), Some(TypeClass::Text));
        assert_eq!(class_of_plc_type("\"limitConfig\""), None);
        assert!(Value::parse(TypeClass::Real, "nan").is_err());
        assert_eq!(Value::parse(TypeClass::Bool, "TRUE").unwrap(), Value::Bool(true));
        assert!(Value::parse(TypeClass::Integer, "1.5").is_err());
    }
}

//! User parameters (Fusion's "Change Parameters" dialog).

use crate::expr::{self, ExprError};

#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub expression: String,
    pub comment: String,
}

#[derive(Clone, Debug, Default)]
pub struct ParamTable {
    params: Vec<Parameter>,
}

impl ParamTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Parameter> {
        self.params.iter()
    }

    pub fn get(&self, name: &str) -> Option<&Parameter> {
        self.params.iter().find(|p| p.name == name)
    }

    /// Create or update a parameter. Names must be identifiers and must not
    /// shadow a unit (`mm`, `in` …).
    pub fn set(&mut self, name: &str, expression: &str) -> Result<(), ExprError> {
        let valid = name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !["mm", "cm", "m", "in", "deg", "rad", "pi"].contains(&name);
        if !valid {
            return Err(ExprError::Syntax(format!("invalid parameter name '{name}'")));
        }
        let old = self.get(name).cloned();
        match self.params.iter_mut().find(|p| p.name == name) {
            Some(p) => p.expression = expression.to_string(),
            None => self.params.push(Parameter {
                name: name.to_string(),
                expression: expression.to_string(),
                comment: String::new(),
            }),
        }
        // Reject edits that break the table (cycles, unknown names).
        if let Err(e) = self.value(name) {
            match old {
                Some(old) => self.params.iter_mut().find(|p| p.name == name).unwrap().expression = old.expression,
                None => self.params.retain(|p| p.name != name),
            }
            return Err(e);
        }
        Ok(())
    }

    pub fn set_comment(&mut self, name: &str, comment: &str) {
        if let Some(p) = self.params.iter_mut().find(|p| p.name == name) {
            p.comment = comment.to_string();
        }
    }

    /// Remove a parameter unless other parameters still use it.
    pub fn remove(&mut self, name: &str) -> Result<(), String> {
        let users: Vec<&str> = self
            .params
            .iter()
            .filter(|p| expr::references(&p.expression).iter().any(|r| r == name))
            .map(|p| p.name.as_str())
            .collect();
        if !users.is_empty() {
            return Err(format!("'{name}' is used by {}", users.join(", ")));
        }
        self.params.retain(|p| p.name != name);
        Ok(())
    }

    pub fn value(&self, name: &str) -> Result<f64, ExprError> {
        self.resolve(name, &mut Vec::new())
    }

    /// Evaluate an arbitrary expression against this table.
    pub fn eval(&self, src: &str) -> Result<f64, ExprError> {
        let mut stack = Vec::new();
        expr::eval(src, &mut |n| self.resolve(n, &mut stack))
    }

    fn resolve(&self, name: &str, stack: &mut Vec<String>) -> Result<f64, ExprError> {
        if stack.iter().any(|s| s == name) {
            return Err(ExprError::Cycle(name.to_string()));
        }
        let Some(p) = self.get(name) else {
            return match name {
                "pi" => Ok(std::f64::consts::PI),
                _ => Err(ExprError::UnknownName(name.to_string())),
            };
        };
        stack.push(name.to_string());
        let src = p.expression.clone();
        let result = expr::eval(&src, &mut |n| self.resolve(n, stack));
        stack.pop();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependent_parameters() {
        let mut t = ParamTable::new();
        t.set("width", "40 mm").unwrap();
        t.set("wall", "2").unwrap();
        t.set("inner", "width - 2 * wall").unwrap();
        assert_eq!(t.value("inner").unwrap(), 36.0);
        t.set("width", "5 cm").unwrap();
        assert_eq!(t.value("inner").unwrap(), 46.0);
        assert_eq!(t.eval("inner / 2").unwrap(), 23.0);
    }

    #[test]
    fn rejects_cycles_and_keeps_old_value() {
        let mut t = ParamTable::new();
        t.set("a", "1").unwrap();
        t.set("b", "a + 1").unwrap();
        assert!(matches!(t.set("a", "b * 2"), Err(ExprError::Cycle(_))));
        assert_eq!(t.value("a").unwrap(), 1.0);
        assert!(t.set("c", "missing").is_err());
        assert!(t.get("c").is_none());
    }

    #[test]
    fn cannot_remove_used_parameter() {
        let mut t = ParamTable::new();
        t.set("a", "1").unwrap();
        t.set("b", "a").unwrap();
        assert!(t.remove("a").is_err());
        t.remove("b").unwrap();
        t.remove("a").unwrap();
    }

    #[test]
    fn invalid_names() {
        let mut t = ParamTable::new();
        assert!(t.set("2x", "1").is_err());
        assert!(t.set("mm", "1").is_err());
        assert!(t.set("my-param", "1").is_err());
    }
}

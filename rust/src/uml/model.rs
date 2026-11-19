//! Data model for the UML class diagram.

/// One attribute of a class, e.g. `x: int`.
#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub ty: Option<String>,
    /// Class-level (static) attribute. Drawn underlined, as in UML.
    pub is_static: bool,
}

/// One parameter of a method, e.g. `p:(int,int)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Option<String>,
}

/// One method of a class, e.g. `move(p:(int,int))`.
#[derive(Clone, Debug, PartialEq)]
pub struct Method {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<String>,
    pub is_static: bool,
    /// 1-based line number of the `def`.
    pub line: usize,
}

/// A class found in the project's source code.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassInfo {
    pub name: String,
    pub bases: Vec<String>,
    pub attributes: Vec<Attribute>,
    pub methods: Vec<Method>,
    /// File name (relative to the project folder) the class is defined in.
    pub file: String,
    /// 1-based line number of the `class` statement.
    pub line: usize,
}

impl Attribute {
    /// UML text, e.g. `x: int` (or just `x` when the type is unknown).
    pub fn display(&self) -> String {
        match &self.ty {
            Some(t) => format!("{}: {}", self.name, t),
            None => self.name.clone(),
        }
    }
}

impl Param {
    pub fn display(&self) -> String {
        match &self.ty {
            Some(t) => format!("{}:{}", self.name, t),
            None => self.name.clone(),
        }
    }
}

impl Method {
    /// UML text, e.g. `move(p:(int,int))` or `area(): float`.
    pub fn display(&self) -> String {
        let params: Vec<String> = self.params.iter().map(Param::display).collect();
        let mut s = format!("{}({})", self.name, params.join(", "));
        if let Some(r) = &self.ret {
            s.push_str(": ");
            s.push_str(r);
        }
        s
    }
}

impl ClassInfo {
    pub fn attribute_mut(&mut self, name: &str) -> Option<&mut Attribute> {
        self.attributes.iter_mut().find(|a| a.name == name)
    }

    pub fn attribute_type(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.name == name)
            .and_then(|a| a.ty.as_deref())
    }
}

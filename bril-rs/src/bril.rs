//! Bril syntax reference spec: https://capra.cs.cornell.edu/bril/lang/syntax.html
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Prog {
    pub functions: Vec<Function>,
}

impl Prog {
    #[inline]
    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Function {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<Arg>>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub instrs: Vec<LabelOrInst>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash)]
pub struct Arg {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(untagged)]
pub enum ValueLit {
    Int(i32),
    Bool(bool),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialOrd, Ord, Eq, PartialEq, Hash)]
#[serde(rename_all = "lowercase", untagged)]
pub enum LabelOrInst {
    Inst {
        op: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dest: Option<String>,

        #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
        ty: Option<String>,

        #[serde(default, skip_serializing_if = "Option::is_none")]
        args: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        funcs: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        labels: Option<Vec<String>>,

        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<ValueLit>,
    },
    Label {
        label: String,
    },
}

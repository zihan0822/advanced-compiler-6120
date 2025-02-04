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
    #[serde(default)]
    pub args: Option<Vec<Arg>>,
    #[serde(default, rename = "type")]
    pub ty: Option<String>,
    pub instrs: Vec<LabelOrInst>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Arg {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase", untagged)]
pub enum LabelOrInst {
    Inst {
        op: String,
        #[serde(default)]
        dest: Option<String>,

        #[serde(default, rename = "type")]
        ty: Option<String>,

        #[serde(default)]
        args: Option<Vec<String>>,
        #[serde(default)]
        funcs: Option<Vec<String>>,
        #[serde(default)]
        labels: Option<Vec<String>>,
    },
    Label {
        label: String,
    },
}

//! Open-Jev request compilation and candidate prompt rendering (port of
//! `jev/api.py::compile_request` and `candidate_prompts`, MIT, Open-Jev contributors).
//!
//! Each candidate becomes an independent Yes/No prompt; the model never sees
//! other candidates, their order, question ids or targets.

use crate::pyjson;
use anyhow::{bail, Result};
use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
use serde_json::Value;

/// A JSON object that keeps key order (Open-Jev candidate order matters for output order).
#[derive(Clone, Debug, Default)]
pub struct Ordered<T>(pub Vec<(String, T)>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Ordered<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = Ordered<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> std::result::Result<Self::Value, A::Error> {
                let mut v = Vec::new();
                while let Some((k, x)) = m.next_entry::<String, T>()? {
                    v.push((k, x));
                }
                Ok(Ordered(v))
            }
        }
        d.deserialize_map(V(std::marker::PhantomData))
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(untagged)]
pub enum Criteria {
    Map(Ordered<Value>),
    List(Vec<Value>),
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct QuestionDef {
    #[serde(rename = "type", default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub instructions: Value,
    #[serde(default)]
    pub criteria: Option<Criteria>,
}
fn default_kind() -> String {
    "choice".into()
}

/// Open-Jev request: shared state plus typed questions (in order).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Request {
    pub state: Value,
    pub questions: Ordered<QuestionDef>,
}

impl Request {
    pub fn compile(&self) -> Result<Vec<(String, Record)>> {
        self.questions.0.iter().map(|(id, d)| Ok((id.clone(), Record::from_definition(&self.state, d)?))).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Choice,
    Score,
    Noul,
}

#[derive(Clone, Debug)]
pub struct Record {
    pub kind: Kind,
    pub state: Value,
    pub question: String,
    /// Rendered option text shown to the model (choice: "name: description").
    pub options: Vec<String>,
    /// Software-only answer keys, in output order.
    pub answer_keys: Vec<String>,
}

fn render(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => pyjson::dumps(other),
    }
}

fn description(v: &Value) -> Result<String> {
    match v {
        Value::String(_) | Value::Object(_) | Value::Array(_) => Ok(render(v)),
        _ => bail!("instructions and descriptions must be text, an object, or an array"),
    }
}

impl Record {
    /// Choice from an ordered list of (name, optional description).
    pub fn choice(state: Value, question: &str, cands: Vec<(String, Option<Value>)>) -> Result<Self> {
        if cands.is_empty() || cands.len() > 255 {
            bail!("Choice requires between 1 and 255 candidates");
        }
        let mut options = Vec::new();
        let mut keys = Vec::new();
        for (name, desc) in cands {
            let opt = match desc {
                None | Some(Value::Null) => name.clone(),
                Some(d) => format!("{}: {}", name, description(&d)?),
            };
            options.push(opt);
            keys.push(name);
        }
        Ok(Record { kind: Kind::Choice, state, question: question.to_string(), options, answer_keys: keys })
    }

    pub fn score(state: Value, question: &str, levels: &[Value]) -> Result<Self> {
        if levels.len() < 2 || levels.len() > 10 {
            bail!("Score requires an array of 2 to 10 descriptive levels");
        }
        let options = levels.iter().map(description).collect::<Result<Vec<_>>>()?;
        let keys = (0..levels.len()).map(|i| i.to_string()).collect();
        Ok(Record { kind: Kind::Score, state, question: question.to_string(), options, answer_keys: keys })
    }

    pub fn noul(state: Value, question: &str, criteria: Option<&serde_json::Map<String, Value>>) -> Result<Self> {
        let mut q = question.to_string();
        if let Some(c) = criteria {
            for k in c.keys() {
                if k != "true" && k != "false" {
                    bail!("Noul criteria must contain only true and false descriptions");
                }
            }
            for (key, label) in [("true", "Yes"), ("false", "No")] {
                if let Some(d) = c.get(key) {
                    q.push_str(&format!("\n{} means: {}", label, description(d)?));
                }
            }
        }
        Ok(Record {
            kind: Kind::Noul,
            state,
            question: q,
            options: vec!["no".into(), "yes".into()],
            answer_keys: vec!["false".into(), "true".into()],
        })
    }

    /// One Open-Jev question definition: {"type", "instructions", "criteria"}.
    pub fn from_definition(state: &Value, def: &QuestionDef) -> Result<Self> {
        let q = description(&def.instructions)?;
        match (def.kind.as_str(), &def.criteria) {
            ("choice", Some(Criteria::Map(m))) => {
                Record::choice(state.clone(), &q, m.0.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect())
            }
            ("score", Some(Criteria::List(levels))) => Record::score(state.clone(), &q, levels),
            ("noul", None) => Record::noul(state.clone(), &q, None),
            ("noul", Some(Criteria::Map(m))) => {
                let mm: serde_json::Map<String, Value> = m.0.iter().cloned().collect();
                Record::noul(state.clone(), &q, Some(&mm))
            }
            (k, _) => bail!("bad question definition (type {k}): choice needs a criteria object, score a list"),
        }
    }

    /// Independent candidate prompts (user-turn content, before the chat template).
    pub fn candidate_prompts(&self) -> Vec<String> {
        let prefix = format!("Context:\n{}\n\nQuestion: {}\n", render(&self.state), self.question);
        if self.kind == Kind::Noul {
            return vec![prefix + "Is the answer to this question yes? Answer Yes or No."];
        }
        self.options
            .iter()
            .map(|o| format!("{prefix}Proposed answer: {o}\nIs this proposed answer correct? Answer Yes or No."))
            .collect()
    }
}

/// Stable softmax with temperature (Open-Jev `metrics.softmax`).
pub fn softmax(logits: &[f32], temperature: f32) -> Vec<f32> {
    let m = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let w: Vec<f64> = logits.iter().map(|&x| (((x - m) / temperature) as f64).exp()).collect();
    let s: f64 = w.iter().sum();
    w.iter().map(|x| (x / s) as f32).collect()
}

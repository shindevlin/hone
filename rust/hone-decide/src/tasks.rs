//! HONE decision task registry. Built-in tasks come from `tasks/hone_tasks.json`;
//! `HONE_DECIDE_TASKS=<file>` adds or overrides tasks at runtime (same schema).

use crate::prompt::Ordered;
use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

const BUILTIN: &str = include_str!("../tasks/hone_tasks.json");

#[derive(Deserialize, Clone, Debug)]
pub struct Task {
    #[serde(rename = "type", default = "choice")]
    pub kind: String,
    pub question: String,
    pub candidates: Ordered<Value>,
}
fn choice() -> String {
    "choice".into()
}

#[derive(Deserialize)]
struct File {
    tasks: Ordered<Task>,
}

pub struct Registry {
    pub tasks: Vec<(String, Task)>,
}

impl Registry {
    pub fn load() -> Result<Self> {
        let mut tasks = serde_json::from_str::<File>(BUILTIN)?.tasks.0;
        if let Ok(p) = std::env::var("HONE_DECIDE_TASKS") {
            let extra: File = serde_json::from_slice(&std::fs::read(&p)?)?;
            for (k, t) in extra.tasks.0 {
                tasks.retain(|(n, _)| n != &k);
                tasks.push((k, t));
            }
        }
        Ok(Registry { tasks })
    }

    pub fn get(&self, name: &str) -> Option<&Task> {
        self.tasks.iter().find(|(n, _)| n == name).map(|(_, t)| t)
    }

    pub fn to_json(&self) -> Value {
        Value::Object(
            self.tasks
                .iter()
                .map(|(n, t)| {
                    let c: serde_json::Map<String, Value> = t.candidates.0.iter().cloned().collect();
                    (n.clone(), serde_json::json!({"type": t.kind, "question": t.question, "candidates": c}))
                })
                .collect(),
        )
    }
}

pub mod llama;
pub mod qwen35;

use anyhow::Result;

pub enum Model {
    Llama(llama::Model),
    Qwen35(qwen35::Model),
}

#[derive(Clone)]
pub enum State {
    Llama(llama::State),
    Qwen35(qwen35::State),
}

impl Model {
    pub fn new_state(&self) -> State {
        match self {
            Model::Llama(m) => State::Llama(m.new_state()),
            Model::Qwen35(m) => State::Qwen35(m.new_state()),
        }
    }

    /// Feed tokens; returns the final-normed hidden state of the last fed token.
    pub fn forward(&self, st: &mut State, ids: &[u32]) -> Result<Vec<f32>> {
        match (self, st) {
            (Model::Llama(m), State::Llama(s)) => m.forward(s, ids),
            (Model::Qwen35(m), State::Qwen35(s)) => m.forward(s, ids),
            _ => anyhow::bail!("state does not belong to this model"),
        }
    }

    /// Score several continuations of one state. Llama batches them; Qwen3.5 runs them in turn.
    pub fn forward_many(&self, st: &State, seqs: &[&[u32]]) -> Result<Vec<Vec<f32>>> {
        match (self, st) {
            (Model::Llama(m), State::Llama(s)) => m.forward_batch(s, seqs),
            _ => seqs.iter().map(|q| self.forward(&mut st.clone(), q)).collect(),
        }
    }

    pub fn hidden_size(&self) -> usize {
        match self {
            Model::Llama(m) => m.hidden_size(),
            Model::Qwen35(m) => m.hidden_size(),
        }
    }

    pub fn max_len(&self) -> usize {
        match self {
            Model::Llama(m) => m.max_len(),
            Model::Qwen35(m) => m.max_len(),
        }
    }
}

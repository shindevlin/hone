use std::fmt;

#[derive(Debug)]
pub enum RuntimeError {
    /// WASM module failed to compile or link
    CompileError(String),
    /// WASM execution trapped (panic, unreachable, OOB)
    ExecutionTrap(String),
    /// Contract panicked with an explicit message
    ContractPanic(String),
    /// Gas limit exceeded
    GasExhausted,
    /// Method not found in dispatch table
    MethodNotFound(String),
    /// Invalid WASM binary
    InvalidWasm(String),
    /// Contract not deployed at given address
    ContractNotFound(String),
    /// Invalid arguments for a method
    InvalidArgs(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CompileError(e) => write!(f, "Compile error: {}", e),
            Self::ExecutionTrap(e) => write!(f, "Execution trap: {}", e),
            Self::ContractPanic(e) => write!(f, "Contract panicked: {}", e),
            Self::GasExhausted => write!(f, "Gas exhausted"),
            Self::MethodNotFound(m) => write!(f, "Method not found: {}", m),
            Self::InvalidWasm(e) => write!(f, "Invalid WASM: {}", e),
            Self::ContractNotFound(a) => write!(f, "Contract not found: {}", a),
            Self::InvalidArgs(e) => write!(f, "Invalid arguments: {}", e),
        }
    }
}

impl std::error::Error for RuntimeError {}

pub mod bits;
pub mod debug;
pub mod escaped_string;
pub mod filesystem;
pub mod number;
pub mod range;
pub mod string;
pub mod expr_value;
pub mod unique_id;
pub mod vec;
pub mod core_utils_error;
pub mod file_object;
pub mod tree;
pub mod log;
pub mod history_var;

pub mod traits {
    pub mod generate_code;
}

pub use expr_value::{CLRObjectComputeResult, ExprValue};
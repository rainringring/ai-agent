use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn calculator(
    operator: &str,
    first_number: f64,
    second_number: f64,
) -> anyhow::Result<f64, String> {
    match operator {
        "add" => Ok(first_number + second_number),
        "subtract" => Ok(first_number - second_number),
        "multiply" => Ok(first_number * second_number),
        "divide" => {
            if second_number == 0.0 {
                Err("Cannot devide by zero".to_string())
            } else {
                Ok(first_number / second_number)
            }
        }
        other => Err(format!("Unsupported operator: {other}")),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CalculatorArgs {
    pub operator: String,
    pub first_number: f64,
    pub second_number: f64,
}
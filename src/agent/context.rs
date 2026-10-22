use std::collections::HashMap;

use serde_json::Value;
use uuid::Uuid;

use crate::agent::event::Event;

#[derive(Debug)]
pub struct ExecutionContext {
    pub execution_id: String,
    pub events: Vec<Event>,
    pub current_step: u32,
    pub state: HashMap<String, Value>,
    pub final_result: Option<String>,
}

impl ExecutionContext {
    pub fn new() -> Self {
        Self {
            execution_id: Uuid::new_v4().to_string(),
            events: Vec::new(),
            current_step: 0,
            state: HashMap::new(),
            final_result: None,
        }
    }

    pub fn add_event(&mut self, event: Event) {
        self.events.push(event);
    }

    pub fn increment_step(&mut self) {
        self.current_step += 1;
    }
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self::new()
    }
}
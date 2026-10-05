//! Typed domain events and the cancellable-task registry (AD-3).
//!
//! Events are past-tense facts named `<area>.<thing>.<verb-ed>`. Long-running
//! work registers a [`TaskHandle`] here, reports `task.progressed` while it
//! runs, observes cancellation through its [`CancellationToken`], and reports
//! `task.finished` when it ends. The UI subscribes to invalidate TanStack Query
//! caches; it never polls.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;
use tauri_specta::Event;
use tokio_util::sync::CancellationToken;

/// Progress for a long task, emitted as `task.progressed`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "task.progressed")]
pub struct TaskProgressed {
    /// Stable id of the task instance (matches the `task.finished` id).
    pub task_id: String,
    /// Owning area, e.g. `agent.run`, `ingest.run`, `apply.attempt`.
    pub area: String,
    /// Units completed so far.
    pub done: u32,
    /// Total units when known.
    pub total: Option<u32>,
    /// Optional i18n key describing the current step (never raw prose).
    pub message_key: Option<String>,
}

/// How a long task ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    /// Ran to completion.
    Completed,
    /// Stopped because its token was cancelled.
    Cancelled,
    /// Stopped with an error.
    Failed,
}

/// Terminal event for a long task, emitted as `task.finished`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "task.finished")]
pub struct TaskFinished {
    /// Id of the task that ended.
    pub task_id: String,
    /// Owning area, e.g. `agent.run`.
    pub area: String,
    /// Terminal outcome.
    pub outcome: TaskOutcome,
    /// Optional i18n key describing the outcome.
    pub message_key: Option<String>,
}

/// Registry of in-flight cancellable tasks, managed as Tauri state.
#[derive(Clone, Default)]
pub struct TaskRegistry {
    tasks: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl TaskRegistry {
    /// Register a task and return its handle.
    pub fn begin(&self, area: impl Into<String>, task_id: impl Into<String>) -> TaskHandle {
        let task_id = task_id.into();
        let token = CancellationToken::new();
        self.tasks
            .lock()
            .unwrap()
            .insert(task_id.clone(), token.clone());
        TaskHandle {
            task_id,
            area: area.into(),
            registry: self.clone(),
            token,
        }
    }

    /// Cancel a task by id. Returns `false` if the id is unknown.
    pub fn cancel(&self, task_id: &str) -> bool {
        match self.tasks.lock().unwrap().get(task_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// Whether a task id is currently registered and cancelled.
    pub fn is_cancelled(&self, task_id: &str) -> bool {
        self.tasks
            .lock()
            .unwrap()
            .get(task_id)
            .is_some_and(CancellationToken::is_cancelled)
    }

    /// Number of in-flight tasks.
    pub fn len(&self) -> usize {
        self.tasks.lock().unwrap().len()
    }

    /// Whether no task is in flight.
    pub fn is_empty(&self) -> bool {
        self.tasks.lock().unwrap().is_empty()
    }

    fn remove(&self, task_id: &str) {
        self.tasks.lock().unwrap().remove(task_id);
    }
}

/// A handle to one registered long task.
#[derive(Clone)]
pub struct TaskHandle {
    task_id: String,
    area: String,
    registry: TaskRegistry,
    token: CancellationToken,
}

impl TaskHandle {
    /// The task id.
    pub fn id(&self) -> &str {
        &self.task_id
    }

    /// The owning area.
    pub fn area(&self) -> &str {
        &self.area
    }

    /// A clone of the cancellation token to poll inside the task.
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Whether this task has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Emit `task.progressed`.
    pub fn progress(
        &self,
        app: &AppHandle,
        done: u32,
        total: Option<u32>,
        message_key: Option<&str>,
    ) {
        let _ = TaskProgressed {
            task_id: self.task_id.clone(),
            area: self.area.clone(),
            done,
            total,
            message_key: message_key.map(str::to_owned),
        }
        .emit(app);
    }

    /// Emit `task.finished` and unregister the task.
    pub fn finished(&self, app: &AppHandle, outcome: TaskOutcome, message_key: Option<&str>) {
        let _ = TaskFinished {
            task_id: self.task_id.clone(),
            area: self.area.clone(),
            outcome,
            message_key: message_key.map(str::to_owned),
        }
        .emit(app);
        self.registry.remove(&self.task_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_marks_registered_task() {
        let registry = TaskRegistry::default();
        let handle = registry.begin("agent.run", "run-1");
        assert_eq!(registry.len(), 1);
        assert!(!handle.is_cancelled());

        assert!(registry.cancel("run-1"));
        assert!(handle.is_cancelled());
        assert!(registry.is_cancelled("run-1"));

        // Unknown ids are a no-op, not an error.
        assert!(!registry.cancel("missing"));
    }

    #[test]
    fn finished_unregisters() {
        let registry = TaskRegistry::default();
        let handle = registry.begin("ingest.run", "run-2");
        registry.remove(handle.id());
        assert!(registry.is_empty());
    }
}

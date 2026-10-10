use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum WorkerStatus {
    Offline,
    Online,
    Busy,
    Error,
}

impl fmt::Display for WorkerStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerStatus::Offline => write!(f, "offline"),
            WorkerStatus::Online => write!(f, "online"),
            WorkerStatus::Busy => write!(f, "busy"),
            WorkerStatus::Error => write!(f, "error"),
        }
    }
}

impl std::str::FromStr for WorkerStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "offline" => Ok(WorkerStatus::Offline),
            "online" => Ok(WorkerStatus::Online),
            "busy" => Ok(WorkerStatus::Busy),
            "error" => Ok(WorkerStatus::Error),
            other => Err(format!("Invalid worker status '{}'", other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum TaskRunStatus {
    Queued,
    Running,
    Cancelling,
    Cancelled,
    Succeeded,
    Failed,
    Lost,
}

impl fmt::Display for TaskRunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskRunStatus::Queued => write!(f, "queued"),
            TaskRunStatus::Running => write!(f, "running"),
            TaskRunStatus::Cancelling => write!(f, "cancelling"),
            TaskRunStatus::Cancelled => write!(f, "cancelled"),
            TaskRunStatus::Succeeded => write!(f, "succeeded"),
            TaskRunStatus::Failed => write!(f, "failed"),
            TaskRunStatus::Lost => write!(f, "lost"),
        }
    }
}

impl std::str::FromStr for TaskRunStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "queued" => Ok(TaskRunStatus::Queued),
            "running" => Ok(TaskRunStatus::Running),
            "cancelling" => Ok(TaskRunStatus::Cancelling),
            "cancelled" => Ok(TaskRunStatus::Cancelled),
            "succeeded" => Ok(TaskRunStatus::Succeeded),
            "failed" => Ok(TaskRunStatus::Failed),
            "lost" => Ok(TaskRunStatus::Lost),
            other => Err(format!("Invalid task run status '{}'", other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum StepResultStatus {
    Success,
    Failed,
    BranchMatched,
    BranchNotMatched,
}

impl fmt::Display for StepResultStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StepResultStatus::Success => write!(f, "success"),
            StepResultStatus::Failed => write!(f, "failed"),
            StepResultStatus::BranchMatched => write!(f, "branch_matched"),
            StepResultStatus::BranchNotMatched => write!(f, "branch_not_matched"),
        }
    }
}

impl std::str::FromStr for StepResultStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "success" => Ok(StepResultStatus::Success),
            "failed" => Ok(StepResultStatus::Failed),
            "branch_matched" => Ok(StepResultStatus::BranchMatched),
            "branch_not_matched" => Ok(StepResultStatus::BranchNotMatched),
            other => Err(format!("Invalid step result status '{}'", other)),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_worker_status_display_from_str_and_serde() {
        // Arrange
        let statuses = [
            (WorkerStatus::Offline, "offline"),
            (WorkerStatus::Online, "online"),
            (WorkerStatus::Busy, "busy"),
            (WorkerStatus::Error, "error"),
        ];

        for (status, expected_str) in statuses {
            // Act & Assert Display
            assert_eq!(status.to_string(), expected_str);

            // Act & Assert FromStr with case and whitespace variations
            let uppercase_with_spaces = format!("  {}  ", expected_str.to_uppercase());
            let parsed =
                WorkerStatus::from_str(&uppercase_with_spaces).expect("parsing valid status");
            assert_eq!(parsed, status);

            // Act & Assert Serde JSON roundtrip
            let json = serde_json::to_string(&status).expect("serialization to JSON");
            let deserialized: WorkerStatus =
                serde_json::from_str(&json).expect("deserialization from JSON");
            assert_eq!(deserialized, status);
        }

        // Act & Assert Invalid Status
        let err = WorkerStatus::from_str("unknown_status").unwrap_err();
        assert_eq!(err, "Invalid worker status 'unknown_status'");
    }

    #[test]
    fn test_task_run_status_display_from_str_and_serde() {
        // Arrange
        let statuses = [
            (TaskRunStatus::Queued, "queued"),
            (TaskRunStatus::Running, "running"),
            (TaskRunStatus::Cancelling, "cancelling"),
            (TaskRunStatus::Cancelled, "cancelled"),
            (TaskRunStatus::Succeeded, "succeeded"),
            (TaskRunStatus::Failed, "failed"),
            (TaskRunStatus::Lost, "lost"),
        ];

        for (status, expected_str) in statuses {
            // Act & Assert Display
            assert_eq!(status.to_string(), expected_str);

            // Act & Assert FromStr with case and whitespace variations
            let mixed_case_with_spaces = format!("\t {} \n", expected_str.to_uppercase());
            let parsed =
                TaskRunStatus::from_str(&mixed_case_with_spaces).expect("parsing valid status");
            assert_eq!(parsed, status);

            // Act & Assert Serde JSON roundtrip
            let json = serde_json::to_string(&status).expect("serialization to JSON");
            let deserialized: TaskRunStatus =
                serde_json::from_str(&json).expect("deserialization from JSON");
            assert_eq!(deserialized, status);
        }

        // Act & Assert Invalid Status
        let err = TaskRunStatus::from_str("pending").unwrap_err();
        assert_eq!(err, "Invalid task run status 'pending'");
    }

    #[test]
    fn test_step_result_status_display_from_str_and_serde() {
        // Arrange
        let statuses = [
            (StepResultStatus::Success, "success"),
            (StepResultStatus::Failed, "failed"),
            (StepResultStatus::BranchMatched, "branch_matched"),
            (StepResultStatus::BranchNotMatched, "branch_not_matched"),
        ];

        for (status, expected_str) in statuses {
            // Act & Assert Display
            assert_eq!(status.to_string(), expected_str);

            // Act & Assert FromStr with case and whitespace variations
            let mixed_case_with_spaces = format!("  {}  ", expected_str.to_uppercase());
            let parsed =
                StepResultStatus::from_str(&mixed_case_with_spaces).expect("parsing valid status");
            assert_eq!(parsed, status);

            // Act & Assert Serde JSON roundtrip
            let json = serde_json::to_string(&status).expect("serialization to JSON");
            let deserialized: StepResultStatus =
                serde_json::from_str(&json).expect("deserialization from JSON");
            assert_eq!(deserialized, status);
        }

        // Act & Assert Invalid Status
        let err = StepResultStatus::from_str("passed").unwrap_err();
        assert_eq!(err, "Invalid step result status 'passed'");
    }
}

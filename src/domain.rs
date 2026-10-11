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

impl WorkerStatus {
    pub fn can_transition_to(&self, target: WorkerStatus) -> bool {
        if *self == target {
            return true;
        }
        match self {
            WorkerStatus::Offline => matches!(target, WorkerStatus::Online | WorkerStatus::Error),
            WorkerStatus::Online => matches!(target, WorkerStatus::Busy | WorkerStatus::Offline | WorkerStatus::Error),
            WorkerStatus::Busy => matches!(target, WorkerStatus::Online | WorkerStatus::Offline | WorkerStatus::Error),
            WorkerStatus::Error => matches!(target, WorkerStatus::Offline | WorkerStatus::Online),
        }
    }
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

impl TaskRunStatus {
    pub fn can_transition_to(&self, target: TaskRunStatus) -> bool {
        if *self == target {
            return true;
        }
        match self {
            TaskRunStatus::Queued => matches!(target, TaskRunStatus::Running | TaskRunStatus::Cancelled),
            TaskRunStatus::Running => matches!(
                target,
                TaskRunStatus::Cancelling
                    | TaskRunStatus::Cancelled
                    | TaskRunStatus::Succeeded
                    | TaskRunStatus::Failed
                    | TaskRunStatus::Lost
            ),
            TaskRunStatus::Cancelling => matches!(
                target,
                TaskRunStatus::Cancelled | TaskRunStatus::Failed | TaskRunStatus::Lost
            ),
            TaskRunStatus::Succeeded | TaskRunStatus::Failed | TaskRunStatus::Cancelled | TaskRunStatus::Lost => false,
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum AutomationStatus {
    Draft,
    Active,
    Archived,
}

impl AutomationStatus {
    pub fn can_transition_to(&self, target: AutomationStatus) -> bool {
        if *self == target {
            return true;
        }
        match self {
            AutomationStatus::Draft => matches!(target, AutomationStatus::Active | AutomationStatus::Archived),
            AutomationStatus::Active => matches!(target, AutomationStatus::Draft | AutomationStatus::Archived),
            AutomationStatus::Archived => false,
        }
    }
}

impl fmt::Display for AutomationStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AutomationStatus::Draft => write!(f, "draft"),
            AutomationStatus::Active => write!(f, "active"),
            AutomationStatus::Archived => write!(f, "archived"),
        }
    }
}

impl std::str::FromStr for AutomationStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "draft" => Ok(AutomationStatus::Draft),
            "active" => Ok(AutomationStatus::Active),
            "archived" => Ok(AutomationStatus::Archived),
            other => Err(format!("Invalid automation status '{}'", other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum RecordingSessionStatus {
    Recording,
    Completed,
    Imported,
    Discarded,
}

impl RecordingSessionStatus {
    pub fn can_transition_to(&self, target: RecordingSessionStatus) -> bool {
        if *self == target {
            return true;
        }
        match self {
            RecordingSessionStatus::Recording => matches!(
                target,
                RecordingSessionStatus::Completed | RecordingSessionStatus::Discarded
            ),
            RecordingSessionStatus::Completed => matches!(
                target,
                RecordingSessionStatus::Imported | RecordingSessionStatus::Discarded
            ),
            RecordingSessionStatus::Imported | RecordingSessionStatus::Discarded => false,
        }
    }
}

impl fmt::Display for RecordingSessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordingSessionStatus::Recording => write!(f, "recording"),
            RecordingSessionStatus::Completed => write!(f, "completed"),
            RecordingSessionStatus::Imported => write!(f, "imported"),
            RecordingSessionStatus::Discarded => write!(f, "discarded"),
        }
    }
}

impl std::str::FromStr for RecordingSessionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "recording" => Ok(RecordingSessionStatus::Recording),
            "completed" => Ok(RecordingSessionStatus::Completed),
            "imported" => Ok(RecordingSessionStatus::Imported),
            "discarded" => Ok(RecordingSessionStatus::Discarded),
            other => Err(format!("Invalid recording session status '{}'", other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduleStatus {
    Disabled,
    Enabled,
}

impl ScheduleStatus {
    pub fn can_transition_to(&self, _target: ScheduleStatus) -> bool {
        true
    }
}

impl fmt::Display for ScheduleStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScheduleStatus::Disabled => write!(f, "disabled"),
            ScheduleStatus::Enabled => write!(f, "enabled"),
        }
    }
}

impl std::str::FromStr for ScheduleStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "disabled" | "false" => Ok(ScheduleStatus::Disabled),
            "enabled" | "true" => Ok(ScheduleStatus::Enabled),
            other => Err(format!("Invalid schedule status '{}'", other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserSessionStatus {
    Active,
    Expired,
    Revoked,
}

impl UserSessionStatus {
    pub fn can_transition_to(&self, target: UserSessionStatus) -> bool {
        if *self == target {
            return true;
        }
        match self {
            UserSessionStatus::Active => matches!(target, UserSessionStatus::Expired | UserSessionStatus::Revoked),
            UserSessionStatus::Expired | UserSessionStatus::Revoked => false,
        }
    }
}

impl fmt::Display for UserSessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UserSessionStatus::Active => write!(f, "active"),
            UserSessionStatus::Expired => write!(f, "expired"),
            UserSessionStatus::Revoked => write!(f, "revoked"),
        }
    }
}

impl std::str::FromStr for UserSessionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "active" => Ok(UserSessionStatus::Active),
            "expired" => Ok(UserSessionStatus::Expired),
            "revoked" => Ok(UserSessionStatus::Revoked),
            other => Err(format!("Invalid user session status '{}'", other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WpsModeStatus {
    Disabled,
    Enabled,
}

impl WpsModeStatus {
    pub fn can_transition_to(&self, _target: WpsModeStatus) -> bool {
        true
    }
}

impl fmt::Display for WpsModeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WpsModeStatus::Disabled => write!(f, "disabled"),
            WpsModeStatus::Enabled => write!(f, "enabled"),
        }
    }
}

impl std::str::FromStr for WpsModeStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "disabled" | "false" => Ok(WpsModeStatus::Disabled),
            "enabled" | "true" => Ok(WpsModeStatus::Enabled),
            other => Err(format!("Invalid WPS mode status '{}'", other)),
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
    fn test_entity_state_machine_transitions() {
        // WorkerStatus transitions
        assert!(WorkerStatus::Offline.can_transition_to(WorkerStatus::Online));
        assert!(WorkerStatus::Online.can_transition_to(WorkerStatus::Busy));
        assert!(WorkerStatus::Busy.can_transition_to(WorkerStatus::Online));
        assert!(WorkerStatus::Busy.can_transition_to(WorkerStatus::Offline));
        assert!(WorkerStatus::Error.can_transition_to(WorkerStatus::Offline));

        // TaskRunStatus transitions
        assert!(TaskRunStatus::Queued.can_transition_to(TaskRunStatus::Running));
        assert!(TaskRunStatus::Running.can_transition_to(TaskRunStatus::Succeeded));
        assert!(TaskRunStatus::Running.can_transition_to(TaskRunStatus::Failed));
        assert!(TaskRunStatus::Running.can_transition_to(TaskRunStatus::Cancelling));
        assert!(TaskRunStatus::Cancelling.can_transition_to(TaskRunStatus::Cancelled));
        assert!(!TaskRunStatus::Succeeded.can_transition_to(TaskRunStatus::Running));

        // AutomationStatus transitions
        assert!(AutomationStatus::Draft.can_transition_to(AutomationStatus::Active));
        assert!(AutomationStatus::Active.can_transition_to(AutomationStatus::Draft));
        assert!(AutomationStatus::Active.can_transition_to(AutomationStatus::Archived));
        assert!(!AutomationStatus::Archived.can_transition_to(AutomationStatus::Active));

        // RecordingSessionStatus transitions
        assert!(RecordingSessionStatus::Recording.can_transition_to(RecordingSessionStatus::Completed));
        assert!(RecordingSessionStatus::Completed.can_transition_to(RecordingSessionStatus::Imported));
        assert!(!RecordingSessionStatus::Imported.can_transition_to(RecordingSessionStatus::Recording));

        // ScheduleStatus & WpsModeStatus transitions
        assert!(ScheduleStatus::Disabled.can_transition_to(ScheduleStatus::Enabled));
        assert!(ScheduleStatus::Enabled.can_transition_to(ScheduleStatus::Disabled));

        assert!(WpsModeStatus::Disabled.can_transition_to(WpsModeStatus::Enabled));
        assert!(WpsModeStatus::Enabled.can_transition_to(WpsModeStatus::Disabled));

        // UserSessionStatus transitions
        assert!(UserSessionStatus::Active.can_transition_to(UserSessionStatus::Expired));
        assert!(UserSessionStatus::Active.can_transition_to(UserSessionStatus::Revoked));
        assert!(!UserSessionStatus::Expired.can_transition_to(UserSessionStatus::Active));
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

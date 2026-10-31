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

macro_rules! impl_enum_str {
    (
        $enum_name:ident, $err_label:expr, {
            $( $( [$($alias:expr),*] => )? $variant:ident => $str_val:expr ),* $(,)?
        }
    ) => {
        impl $enum_name {
            #[inline]
            pub fn as_str(&self) -> &'static str {
                match self {
                    $( Self::$variant => $str_val, )*
                }
            }
        }

        impl fmt::Display for $enum_name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::str::FromStr for $enum_name {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let trimmed = s.trim();
                $(
                    if trimmed.eq_ignore_ascii_case($str_val)
                        $( || $( trimmed.eq_ignore_ascii_case($alias) )||* )?
                    {
                        return Ok(Self::$variant);
                    }
                )*
                Err(format!("Invalid {} '{}'", $err_label, trimmed))
            }
        }
    };
}

impl WorkerStatus {
    pub fn can_transition_to(&self, target: WorkerStatus) -> bool {
        if *self == target {
            return true;
        }
        match self {
            WorkerStatus::Offline => matches!(target, WorkerStatus::Online | WorkerStatus::Error),
            WorkerStatus::Online => matches!(
                target,
                WorkerStatus::Busy | WorkerStatus::Offline | WorkerStatus::Error
            ),
            WorkerStatus::Busy => matches!(
                target,
                WorkerStatus::Online | WorkerStatus::Offline | WorkerStatus::Error
            ),
            WorkerStatus::Error => matches!(target, WorkerStatus::Offline | WorkerStatus::Online),
        }
    }
}

impl_enum_str!(WorkerStatus, "worker status", {
    Offline => "offline",
    Online => "online",
    Busy => "busy",
    Error => "error",
});

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
            TaskRunStatus::Queued => {
                matches!(target, TaskRunStatus::Running | TaskRunStatus::Cancelled)
            }
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
            TaskRunStatus::Succeeded
            | TaskRunStatus::Failed
            | TaskRunStatus::Cancelled
            | TaskRunStatus::Lost => false,
        }
    }
}

impl_enum_str!(TaskRunStatus, "task run status", {
    Queued => "queued",
    Running => "running",
    Cancelling => "cancelling",
    Cancelled => "cancelled",
    Succeeded => "succeeded",
    Failed => "failed",
    Lost => "lost",
});

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
            AutomationStatus::Draft => matches!(
                target,
                AutomationStatus::Active | AutomationStatus::Archived
            ),
            AutomationStatus::Active => {
                matches!(target, AutomationStatus::Draft | AutomationStatus::Archived)
            }
            AutomationStatus::Archived => false,
        }
    }
}

impl_enum_str!(AutomationStatus, "automation status", {
    Draft => "draft",
    Active => "active",
    Archived => "archived",
});

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

impl_enum_str!(RecordingSessionStatus, "recording session status", {
    Recording => "recording",
    Completed => "completed",
    Imported => "imported",
    Discarded => "discarded",
});

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

impl_enum_str!(ScheduleStatus, "schedule status", {
    ["false"] => Disabled => "disabled",
    ["true"] => Enabled => "enabled",
});

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
            UserSessionStatus::Active => matches!(
                target,
                UserSessionStatus::Expired | UserSessionStatus::Revoked
            ),
            UserSessionStatus::Expired | UserSessionStatus::Revoked => false,
        }
    }
}

impl_enum_str!(UserSessionStatus, "user session status", {
    Active => "active",
    Expired => "expired",
    Revoked => "revoked",
});

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

impl_enum_str!(WpsModeStatus, "WPS mode status", {
    ["false"] => Disabled => "disabled",
    ["true"] => Enabled => "enabled",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum StepResultStatus {
    Success,
    Failed,
    BranchMatched,
    BranchNotMatched,
}

impl_enum_str!(StepResultStatus, "step result status", {
    Success => "success",
    Failed => "failed",
    BranchMatched => "branch_matched",
    BranchNotMatched => "branch_not_matched",
});

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
        assert!(
            RecordingSessionStatus::Recording.can_transition_to(RecordingSessionStatus::Completed)
        );
        assert!(
            RecordingSessionStatus::Completed.can_transition_to(RecordingSessionStatus::Imported)
        );
        assert!(
            !RecordingSessionStatus::Imported.can_transition_to(RecordingSessionStatus::Recording)
        );

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

    #[test]
    fn test_remaining_domain_statuses_display_from_str_and_serde() {
        // AutomationStatus
        let auto_statuses = [
            (AutomationStatus::Draft, "draft"),
            (AutomationStatus::Active, "active"),
            (AutomationStatus::Archived, "archived"),
        ];
        for (status, expected_str) in auto_statuses {
            assert_eq!(status.to_string(), expected_str);
            let input = format!("  {}  ", expected_str.to_uppercase());
            assert_eq!(AutomationStatus::from_str(&input).unwrap(), status);
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(
                serde_json::from_str::<AutomationStatus>(&json).unwrap(),
                status
            );
        }
        assert_eq!(
            AutomationStatus::from_str("invalid_status").unwrap_err(),
            "Invalid automation status 'invalid_status'"
        );

        // RecordingSessionStatus
        let rec_statuses = [
            (RecordingSessionStatus::Recording, "recording"),
            (RecordingSessionStatus::Completed, "completed"),
            (RecordingSessionStatus::Imported, "imported"),
            (RecordingSessionStatus::Discarded, "discarded"),
        ];
        for (status, expected_str) in rec_statuses {
            assert_eq!(status.to_string(), expected_str);
            let input = format!("\t{}\n", expected_str.to_uppercase());
            assert_eq!(RecordingSessionStatus::from_str(&input).unwrap(), status);
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(
                serde_json::from_str::<RecordingSessionStatus>(&json).unwrap(),
                status
            );
        }
        assert_eq!(
            RecordingSessionStatus::from_str("paused").unwrap_err(),
            "Invalid recording session status 'paused'"
        );

        // ScheduleStatus (supports "disabled"/"false", "enabled"/"true")
        let sched_statuses = [
            (ScheduleStatus::Disabled, "disabled"),
            (ScheduleStatus::Enabled, "enabled"),
        ];
        for (status, expected_str) in sched_statuses {
            assert_eq!(status.to_string(), expected_str);
            let input = format!("  {}  ", expected_str.to_uppercase());
            assert_eq!(ScheduleStatus::from_str(&input).unwrap(), status);
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(
                serde_json::from_str::<ScheduleStatus>(&json).unwrap(),
                status
            );
        }
        assert_eq!(
            ScheduleStatus::from_str(" false ").unwrap(),
            ScheduleStatus::Disabled
        );
        assert_eq!(
            ScheduleStatus::from_str(" TRUE ").unwrap(),
            ScheduleStatus::Enabled
        );
        assert_eq!(
            ScheduleStatus::from_str("unknown").unwrap_err(),
            "Invalid schedule status 'unknown'"
        );

        // UserSessionStatus
        let session_statuses = [
            (UserSessionStatus::Active, "active"),
            (UserSessionStatus::Expired, "expired"),
            (UserSessionStatus::Revoked, "revoked"),
        ];
        for (status, expected_str) in session_statuses {
            assert_eq!(status.to_string(), expected_str);
            let input = format!("  {}  ", expected_str.to_uppercase());
            assert_eq!(UserSessionStatus::from_str(&input).unwrap(), status);
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(
                serde_json::from_str::<UserSessionStatus>(&json).unwrap(),
                status
            );
        }
        assert_eq!(
            UserSessionStatus::from_str("terminated").unwrap_err(),
            "Invalid user session status 'terminated'"
        );

        // WpsModeStatus (supports "disabled"/"false", "enabled"/"true")
        let wps_statuses = [
            (WpsModeStatus::Disabled, "disabled"),
            (WpsModeStatus::Enabled, "enabled"),
        ];
        for (status, expected_str) in wps_statuses {
            assert_eq!(status.to_string(), expected_str);
            let input = format!("  {}  ", expected_str.to_uppercase());
            assert_eq!(WpsModeStatus::from_str(&input).unwrap(), status);
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(
                serde_json::from_str::<WpsModeStatus>(&json).unwrap(),
                status
            );
        }
        assert_eq!(
            WpsModeStatus::from_str(" FALSE ").unwrap(),
            WpsModeStatus::Disabled
        );
        assert_eq!(
            WpsModeStatus::from_str(" true ").unwrap(),
            WpsModeStatus::Enabled
        );
        assert_eq!(
            WpsModeStatus::from_str("invalid").unwrap_err(),
            "Invalid WPS mode status 'invalid'"
        );
    }
}

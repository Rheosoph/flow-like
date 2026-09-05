pub mod calendar;
pub mod copilot;
pub mod excel;
pub mod graph;
pub mod onedrive;
pub mod onenote;
pub mod outlook;
pub mod planner;
pub mod provider;
pub mod sharepoint;
pub mod teams;
pub mod todo;

// Re-export types for external use
pub use calendar::{Calendar, CalendarEvent, MeetingTimeSuggestion};
pub use copilot::{
    ActionItem, CopilotInteraction, GraphSearchHit, GraphSearchResource, MeetingInsight,
    MeetingNote,
};
pub use excel::ExcelWorksheet;
pub use onedrive::{OneDriveItem, OneDriveParentReference};
pub use onenote::{OneNoteNotebook, OneNotePage, OneNoteSection};
pub use outlook::{OutlookCalendarEvent, OutlookContact, OutlookMailFolder, OutlookMessage};
pub use planner::{PlannerBucket, PlannerPlan, PlannerTask};
pub use provider::MicrosoftGraphProvider;
pub use sharepoint::{
    SharePointDrive, SharePointDriveItem, SharePointList, SharePointListItem, SharePointSite,
};
pub use teams::{Channel, ChatMessage, Team};
pub use todo::{TodoTask, TodoTaskList};

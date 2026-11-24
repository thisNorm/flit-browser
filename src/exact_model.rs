use serde::{Deserialize, Serialize};
use windows_sys::Win32::{Foundation::HWND, UI::WindowsAndMessaging::WM_APP};

pub const ACTION_MESSAGE: u32 = WM_APP + 18;
pub const SETTINGS_CONTROL: usize = 211;

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct Surface {
    pub title: String,
    pub subtitle: String,
    pub dark: bool,
    pub compact: bool,
    pub start: bool,
    #[serde(default)]
    pub preserve_input_focus: bool,
    pub sections: Vec<Section>,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub actions: Vec<Action>,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct Action {
    pub id: usize,
    pub label: String,
    pub selected: bool,
    pub kind: ActionKind,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionKind {
    Link,
    Toggle,
    Choice,
}

pub type CreateSurface = unsafe extern "system" fn(HWND, HWND, usize, *const u8, usize) -> HWND;
pub type UpdateSurface = unsafe extern "system" fn(HWND, *const u8, usize) -> i32;

impl Item {
    pub(crate) fn link(id: usize, label: String, detail: String) -> Self {
        Self {
            id: id.to_string(),
            title: String::new(),
            detail,
            actions: vec![Action {
                id,
                label,
                selected: false,
                kind: ActionKind::Link,
            }],
        }
    }
}

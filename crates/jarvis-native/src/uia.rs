// UI Automation (partie « ui* » de tools/desktop-watch.cs) : éléments d'une fenêtre, clic par le
// nom, saisie dans un champ. Un bouton se trouve par son nom, quelle que soit sa position ; le clic
// aux coordonnées n'est qu'un dernier recours.

use std::collections::VecDeque;
use std::time::Instant;

use serde_json::{json, Map, Value};
use unicode_normalization::UnicodeNormalization;
use windows::core::{Interface, BSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Accessibility::*;

fn automation() -> Option<IUIAutomation> {
    crate::com_init();
    unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() }
}

fn type_name(id: UIA_CONTROLTYPE_ID) -> &'static str {
    match id {
        UIA_ButtonControlTypeId => "Button",
        UIA_CalendarControlTypeId => "Calendar",
        UIA_CheckBoxControlTypeId => "CheckBox",
        UIA_ComboBoxControlTypeId => "ComboBox",
        UIA_EditControlTypeId => "Edit",
        UIA_HyperlinkControlTypeId => "Hyperlink",
        UIA_ImageControlTypeId => "Image",
        UIA_ListItemControlTypeId => "ListItem",
        UIA_ListControlTypeId => "List",
        UIA_MenuControlTypeId => "Menu",
        UIA_MenuBarControlTypeId => "MenuBar",
        UIA_MenuItemControlTypeId => "MenuItem",
        UIA_ProgressBarControlTypeId => "ProgressBar",
        UIA_RadioButtonControlTypeId => "RadioButton",
        UIA_ScrollBarControlTypeId => "ScrollBar",
        UIA_SliderControlTypeId => "Slider",
        UIA_SpinnerControlTypeId => "Spinner",
        UIA_StatusBarControlTypeId => "StatusBar",
        UIA_TabControlTypeId => "Tab",
        UIA_TabItemControlTypeId => "TabItem",
        UIA_TextControlTypeId => "Text",
        UIA_ToolBarControlTypeId => "ToolBar",
        UIA_ToolTipControlTypeId => "ToolTip",
        UIA_TreeControlTypeId => "Tree",
        UIA_TreeItemControlTypeId => "TreeItem",
        UIA_CustomControlTypeId => "Custom",
        UIA_GroupControlTypeId => "Group",
        UIA_ThumbControlTypeId => "Thumb",
        UIA_DataGridControlTypeId => "DataGrid",
        UIA_DataItemControlTypeId => "DataItem",
        UIA_DocumentControlTypeId => "Document",
        UIA_SplitButtonControlTypeId => "SplitButton",
        UIA_WindowControlTypeId => "Window",
        UIA_PaneControlTypeId => "Pane",
        UIA_HeaderControlTypeId => "Header",
        UIA_HeaderItemControlTypeId => "HeaderItem",
        UIA_TableControlTypeId => "Table",
        UIA_TitleBarControlTypeId => "TitleBar",
        UIA_SeparatorControlTypeId => "Separator",
        UIA_SemanticZoomControlTypeId => "SemanticZoom",
        UIA_AppBarControlTypeId => "AppBar",
        _ => "",
    }
}

const ACTIONABLE: &[&str] = &["Button", "MenuItem", "Hyperlink", "ListItem", "TabItem", "CheckBox", "RadioButton", "SplitButton", "TreeItem", "DataItem", "ComboBox", "MenuBarItem"];

fn bstr(r: windows::core::Result<BSTR>) -> String {
    r.map(|b| b.to_string()).unwrap_or_default()
}

struct El {
    e: IUIAutomationElement,
}

impl El {
    fn name(&self) -> String {
        unsafe { bstr(self.e.CurrentName()) }
    }
    fn auto_id(&self) -> String {
        unsafe { bstr(self.e.CurrentAutomationId()) }
    }
    fn typ(&self) -> &'static str {
        unsafe { self.e.CurrentControlType().map(type_name).unwrap_or("") }
    }
    fn enabled(&self) -> bool {
        unsafe { self.e.CurrentIsEnabled().map(|b| b.as_bool()).unwrap_or(true) }
    }
    fn rect(&self) -> (i64, i64, i64, i64) {
        unsafe {
            let r = self.e.CurrentBoundingRectangle().unwrap_or_default();
            (r.left as i64, r.top as i64, (r.right - r.left) as i64, (r.bottom - r.top) as i64)
        }
    }
}

// Parcours en largeur des éléments visibles (profondeur et nombre bornés : une appli lente ne bloque pas).
fn elements(h: HWND, max: usize) -> Vec<El> {
    let mut list = vec![];
    let Some(a) = automation() else { return list };
    unsafe {
        let Ok(root) = a.ElementFromHandle(h) else { return list };
        let Ok(walker) = a.ControlViewWalker() else { return list };
        let mut queue: VecDeque<(IUIAutomationElement, u32)> = VecDeque::new();
        queue.push_back((root, 0));
        let started = Instant::now();
        while let Some((item, depth)) = queue.pop_front() {
            if list.len() >= max || started.elapsed().as_millis() >= 4000 {
                break;
            }
            let mut child = walker.GetFirstChildElement(&item).ok();
            while let Some(c) = child {
                if list.len() >= max {
                    break;
                }
                let offscreen = c.CurrentIsOffscreen().map(|b| b.as_bool()).unwrap_or(true);
                if !offscreen {
                    list.push(El { e: c.clone() });
                }
                if depth < 14 {
                    queue.push_back((c.clone(), depth + 1));
                }
                child = walker.GetNextSiblingElement(&c).ok();
            }
        }
    }
    list
}

pub fn norm(s: &str) -> String {
    let d: String = s.replace('&', "").nfd().filter(|c| !('\u{0300}'..='\u{036f}').contains(c)).flat_map(|c| c.to_lowercase()).collect();
    d.split([' ', '\t', '\n', '\r', '.', '…', ':', '\u{a0}']).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

fn describe(e: &El, i: usize) -> Value {
    let (x, y, w, h) = e.rect();
    json!({ "i": i, "name": e.name(), "type": e.typ(), "id": e.auto_id(), "enabled": e.enabled(), "x": x, "y": y, "w": w, "h": h })
}

fn ok(ok: bool, error: Option<&str>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("ok".into(), json!(ok));
    if let Some(e) = error {
        m.insert("error".into(), json!(e));
    }
    m
}

/// Onglets d'une fenêtre (Windows Terminal) : (nom, x, y, largeur, hauteur, sélectionné), coordonnées d'écran.
pub fn tab_items(h: HWND) -> Vec<(String, i64, i64, i64, i64, bool)> {
    elements(h, 3000)
        .into_iter()
        .filter(|e| e.typ() == "TabItem")
        .map(|e| {
            let (x, y, w, hh) = e.rect();
            let selected = unsafe {
                e.e.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
                    .and_then(|p| p.CurrentIsSelected())
                    .map(|b| b.as_bool())
                    .unwrap_or(false)
            };
            (e.name(), x, y, w, hh, selected)
        })
        .collect()
}

pub fn ui_find(h: HWND, limit: usize) -> Map<String, Value> {
    let all = elements(h, 3000);
    let mut found = vec![];
    for e in &all {
        let name = e.name();
        let typ = e.typ();
        if name.trim().is_empty() || (!ACTIVE(typ) && typ != "Edit") {
            continue;
        }
        found.push(describe(e, found.len()));
        if found.len() >= limit {
            break;
        }
    }
    let mut m = ok(true, None);
    m.insert("elements".into(), Value::Array(found));
    m.insert("scanned".into(), json!(all.len()));
    m
}

#[allow(non_snake_case)]
fn ACTIVE(t: &str) -> bool {
    ACTIONABLE.contains(&t)
}

fn best(h: HWND, wanted: &str, kind: &dyn Fn(&str) -> bool, index: i64) -> Option<El> {
    let want = norm(wanted);
    let all = elements(h, 3000);
    if index >= 0 {
        let acts: Vec<El> = all.into_iter().filter(|e| !e.name().trim().is_empty() && kind(e.typ())).collect();
        return acts.into_iter().nth(index as usize);
    }
    let mut best: Option<El> = None;
    let mut best_score = 0;
    for e in all {
        let name = norm(&e.name());
        let id = norm(&e.auto_id());
        let mut score = if name == want || id == want {
            30
        } else if name.starts_with(&format!("{want} ")) || name.ends_with(&format!(" {want}")) {
            20
        } else if want.chars().count() >= 3 && name.contains(&want) {
            12
        } else {
            0
        };
        if score == 0 {
            continue;
        }
        if kind(e.typ()) {
            score += 5;
        }
        if !e.enabled() {
            score -= 8;
        }
        if score > best_score {
            best_score = score;
            best = Some(e);
        }
    }
    best
}

pub fn ui_invoke(h: HWND, name: &str, index: i64) -> Map<String, Value> {
    let Some(e) = best(h, name, &|t| ACTIVE(t), index) else { return ok(false, Some("élément introuvable par l'accessibilité")) };
    let label = e.name();
    let typ = e.typ();
    let mut how = "";
    unsafe {
        if let Ok(p) = e.e.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
            if p.Invoke().is_ok() {
                how = "invoke";
            }
        }
        if how.is_empty() {
            if let Ok(p) = e.e.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) {
                if p.Toggle().is_ok() {
                    how = "toggle";
                }
            }
        }
        if how.is_empty() {
            if let Ok(p) = e.e.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId) {
                if p.Select().is_ok() {
                    how = "select";
                }
            }
        }
        if how.is_empty() {
            if let Ok(p) = e.e.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId) {
                if p.Expand().is_ok() {
                    how = "expand";
                }
            }
        }
    }
    if how.is_empty() {
        let (x, y, w, hh) = e.rect();
        if w <= 0 {
            return ok(false, Some("élément trouvé mais impossible à activer"));
        }
        crate::desktop::activate(h);
        crate::desktop::click_at((x + w / 2) as i32, (y + hh / 2) as i32);
        how = "clic";
    }
    crate::desktop::mark_dirty();
    let mut m = ok(true, None);
    m.insert("name".into(), json!(label));
    m.insert("type".into(), json!(typ));
    m.insert("how".into(), json!(how));
    m
}

pub fn ui_set_value(h: HWND, field: &str, value: &str) -> Map<String, Value> {
    let Some(e) = best(h, field, &|t| t == "Edit" || t == "ComboBox" || t == "Document", -1) else { return ok(false, Some("champ introuvable")) };
    unsafe {
        if e.e.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(false) {
            return ok(false, Some("champ de mot de passe : refusé"));
        }
        let Ok(p) = e.e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) else {
            return ok(false, Some("ce champ ne se remplit pas automatiquement"));
        };
        if let Err(err) = p.SetValue(&BSTR::from(value)) {
            return ok(false, Some(&err.message().to_string()));
        }
    }
    let mut m = ok(true, None);
    m.insert("name".into(), json!(e.name()));
    m
}

#[allow(dead_code)]
fn _cast(e: &IUIAutomationElement) -> Option<windows::core::IUnknown> {
    e.cast().ok()
}

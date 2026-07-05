//! The typed screen model produced by extraction — what the GUI renders.
//!
//! A [`ScreenModule`] is a small arena of [`Node`]s (menus and screens) plus navigation
//! edges. All display strings are the raw CP1251 text from the `.ipo` (the source-language
//! default); the GUI wraps each in its i18n `tr()` at render time.

/// Index into [`ScreenModule::nodes`].
pub type NodeId = usize;

/// A parsed ECU screen module: the full page tree for one SGBD.
#[derive(Debug, Clone)]
pub struct ScreenModule {
    /// SGBD name the screens drive (best-effort; may be empty). This is the FIRST
    /// SGBD literal in the script — only correct for single-variant ECUs. For
    /// multi-variant ECUs prefer [`ScreenModule::group_files`] variant identification.
    pub sgbd: String,
    /// Address-keyed EDIABAS group files referenced by the `.ipo` (`D_<ADDR>`, e.g.
    /// `D_0080`), the INPA variant-identification entry points: open
    /// `ecu/<name>.GRP`, run job `IDENTIFIKATION`, read result `VARIANTE` → the
    /// concrete variant SGBD to load. Usually one; sometimes several (a script
    /// spanning two diagnostic addresses), tried in order. Empty when the script has
    /// no group reference (single-variant ECU → use `sgbd` directly).
    pub group_files: Vec<String>,
    /// INPA script name (usually the `.ipo` file stem).
    pub script: String,
    /// Entry menu (`m_main`).
    pub root: NodeId,
    pub nodes: Vec<Node>,
}

impl ScreenModule {
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }
    pub fn root_menu(&self) -> &Menu {
        match &self.nodes[self.root] {
            Node::Menu(m) => m,
            Node::Screen(_) => unreachable!("root is always a menu"),
        }
    }
    pub fn as_menu(&self, id: NodeId) -> Option<&Menu> {
        match &self.nodes[id] {
            Node::Menu(m) => Some(m),
            _ => None,
        }
    }
    pub fn as_screen(&self, id: NodeId) -> Option<&Screen> {
        match &self.nodes[id] {
            Node::Screen(s) => Some(s),
            _ => None,
        }
    }

    /// Resolve a screen to its variant-specific sibling for `variant` (the connected SGBD),
    /// per INPA's convention `<name>_<variant lowercased>` — e.g. `s_analog1` + `D40M57A1` →
    /// `s_analog1_d40m57a1`. Data items store the generic base screen; the concrete variant
    /// (whose result names may differ, e.g. `_IST_WERT` vs `_WERT`) is chosen here at open
    /// time so the polled names match the ECU actually opened. Falls back to `id` when there
    /// is no such sibling (base ECU) or `variant` is empty.
    pub fn screen_for_variant(&self, id: NodeId, variant: &str) -> NodeId {
        let variant = variant.trim();
        if variant.is_empty() {
            return id;
        }
        let Some(base) = self.as_screen(id) else { return id };
        let want = format!("{}_{}", base.name, variant).to_ascii_lowercase();
        self.nodes
            .iter()
            .position(|n| matches!(n, Node::Screen(s) if s.name.eq_ignore_ascii_case(&want)))
            .unwrap_or(id)
    }
}

/// A node in the page tree.
#[derive(Debug, Clone)]
pub enum Node {
    Menu(Menu),
    Screen(Screen),
}

/// A menu (`m_*`) — a list of navigation items that double as F-keys.
#[derive(Debug, Clone)]
pub struct Menu {
    pub name: String,
    /// Menu title (from `setmenutitle`), if any.
    pub title: String,
    pub items: Vec<NavItem>,
}

/// One menu item / F-key.
#[derive(Debug, Clone)]
pub struct NavItem {
    /// F-key number 1..=10.
    pub fkey: u8,
    /// True when this is Shift+F<n> (source item number > 10).
    pub shifted: bool,
    pub label: String,
    pub target: NavTarget,
}

/// Where a [`NavItem`] leads.
#[derive(Debug, Clone)]
pub enum NavTarget {
    /// Open a screen node.
    Screen(NodeId),
    /// Open a screen and swap the active menu (detail page + its F-key bar).
    ScreenAndMenu { screen: NodeId, menu: NodeId },
    /// Swap the active menu only.
    Menu(NodeId),
    /// Run a job (an activation item — `STEUERN_*`).
    Job(JobCall),
    /// Jump to an external INPA script (cross-ECU / selection).
    Script(String),
    /// Leave the screen.
    Exit,
    /// Go back one level.
    Back,
    /// A recognised-but-unmodelled action (builtin name).
    Other(String),
}

/// A screen (`s_*`) — a titled page of rows.
#[derive(Debug, Clone)]
pub struct Screen {
    pub name: String,
    pub title: String,
    pub kind: ScreenKind,
    /// A single feeder job run once for all rows (e.g. `STATUS_DIGITAL`, `ident`).
    pub feeder: Option<JobCall>,
    pub rows: Vec<Row>,
    /// Static `ftextout` lines for info-only screens (e.g. `s_info`): an "about the SGBD"
    /// page of `label : value` pairs, present when there are no job-bound `rows`.
    pub info: Vec<InfoLine>,
}

/// One static informational line from an `ftextout`-only screen.
#[derive(Debug, Clone, PartialEq)]
pub struct InfoLine {
    pub label: String,
    pub value: InfoValue,
}

/// The value side of an [`InfoLine`].
#[derive(Debug, Clone, PartialEq)]
pub enum InfoValue {
    /// A section header — label only, no value column.
    Heading,
    /// A `label :` field whose value is a runtime SGBD variable we can't resolve
    /// statically (rendered as a dash).
    Runtime,
    /// A literal constant value printed by the SGBD.
    Const(String),
}

/// The archetype of a screen, deciding how the GUI renders it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenKind {
    /// Live measurement page (logical + analog rows).
    DataStream,
    /// Textual info page (identification, coding).
    TextInfo,
    /// Activation page (buttons that run `STEUERN_*` jobs).
    Activation,
    /// Fault-memory page (read `FS_LESEN` / clear `FS_LOESCHEN`).
    Faults,
    /// A screen that is really just a submenu host.
    Menu,
    Other,
}

impl Row {
    /// The row's display label.
    pub fn label(&self) -> &str {
        match self {
            Row::Logical { label, .. }
            | Row::Analog { label, .. }
            | Row::Text { label, .. }
            | Row::Action { label, .. } => label,
        }
    }
    /// The job that produces this row's value/action.
    pub fn job(&self) -> &JobCall {
        match self {
            Row::Logical { job, .. }
            | Row::Analog { job, .. }
            | Row::Text { job, .. }
            | Row::Action { job, .. } => job,
        }
    }
    /// The bound result name, if the row reads one.
    pub fn result(&self) -> Option<&str> {
        match self {
            Row::Logical { result, .. } | Row::Analog { result, .. } | Row::Text { result, .. } => {
                Some(result)
            }
            Row::Action { .. } => None,
        }
    }
    /// The source line group (`.ipo` LINE-record index). Rows sharing a `line` were on
    /// one INPA line (typically a pair) and should render together, in ascending order.
    pub fn line(&self) -> u16 {
        match self {
            Row::Logical { line, .. }
            | Row::Analog { line, .. }
            | Row::Text { line, .. }
            | Row::Action { line, .. } => *line,
        }
    }
}

/// A displayed row, bound to an EDIABAS result.
#[derive(Debug, Clone)]
pub enum Row {
    /// Boolean indicator (On/Off).
    Logical {
        label: String,
        result: String,
        on: String,
        off: String,
        job: JobCall,
        line: u16,
    },
    /// Numeric value shown as a gauge/bar.
    Analog {
        label: String,
        result: String,
        unit: String,
        scale: Scale,
        job: JobCall,
        line: u16,
    },
    /// A textual result (part number, index, …).
    Text {
        label: String,
        result: String,
        job: JobCall,
        line: u16,
    },
    /// An activation action (button → job).
    Action { label: String, job: JobCall, line: u16 },
}

/// An EDIABAS job invocation. `selector` is set when the job is `MW_SELECT_LESEN_NORM`
/// and `arg` is a 4-hex measurement selector.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobCall {
    pub job: String,
    pub arg: String,
    pub selector: Option<String>,
}

/// Linear rescale `value * factor + offset`, plus the display range `[min, max]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub factor: f64,
    pub offset: f64,
    pub min: f64,
    pub max: f64,
}

impl Default for Scale {
    fn default() -> Self {
        Scale { factor: 1.0, offset: 0.0, min: 0.0, max: 0.0 }
    }
}

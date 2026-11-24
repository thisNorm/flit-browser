//! UI Automation for a Win32 client HWND that paints a small Exact surface.

use std::{
    collections::BTreeMap,
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering},
};

use accesskit::{
    Action, ActionHandler, ActionRequest, ActivationHandler, Affine, AriaCurrent, Node, NodeId,
    Rect, Role, Toggled, TreeId, TreeInfo, TreeUpdate,
};
use accesskit_windows::{Adapter, QueuedEvents};
use exact_kernel::PropId;
use exact_painted::{
    Presenter,
    presenter::access::{Node as Source, act, flag, role},
};
use exact_runner::DataSource;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

const ROOT: NodeId = NodeId(0);
const ACTION_BITS: usize = 2;
const ACTION_MASK: usize = (1 << ACTION_BITS) - 1;
static NEXT_INSTANCE: AtomicUsize = AtomicUsize::new(1);

#[derive(PartialEq)]
pub(crate) struct Snapshot {
    nodes: Vec<(NodeId, Node)>,
    focus: NodeId,
}

impl Snapshot {
    /// Capture on the owner thread after layout. Use the painter's client pixels and DPI scale.
    pub(crate) fn capture<D: DataSource>(
        presenter: &mut Presenter<D>,
        title: &str,
        client_px: (i32, i32),
        scale: f32,
    ) -> Self {
        // Exact's focus and action APIs use the last tree published by reader_access.
        presenter.reader_access(1., false);
        let sources = presenter.access_told();
        let mut children = BTreeMap::<i32, Vec<NodeId>>::new();
        for source in sources {
            children
                .entry(source.parent)
                .or_default()
                .push(NodeId(source.id as u64));
        }

        let scale = if scale.is_finite() && scale > 0. {
            f64::from(scale)
        } else {
            1.
        };
        let mut root = Node::new(Role::Window);
        root.set_label(title);
        root.set_bounds(Rect::new(
            0.,
            0.,
            f64::from(client_px.0.max(0)) / scale,
            f64::from(client_px.1.max(0)) / scale,
        ));
        root.set_transform(Affine::scale(scale));
        root.set_children(children.remove(&0).unwrap_or_default());
        let mut nodes = Vec::with_capacity(sources.len() + 1);
        nodes.push((ROOT, root));

        let origin = presenter.access_origin();
        let kernel = presenter.host().kernel();
        for source in sources {
            let props = kernel.node(source.view).map(|node| node.props);
            let mut node = map_node(source, origin);
            if source.role == role::HEADING
                && let Some(level) = props.and_then(|props| props.heading())
                && let Ok(level) = usize::try_from(level - 1)
            {
                node.set_level(level);
            }
            let current = match props
                .as_ref()
                .and_then(|props| props.str(PropId::AccessibilityCurrent))
            {
                Some("true") => AriaCurrent::True,
                Some("page") => AriaCurrent::Page,
                Some("step") => AriaCurrent::Step,
                Some("location") => AriaCurrent::Location,
                Some("date") => AriaCurrent::Date,
                Some("time") => AriaCurrent::Time,
                _ => AriaCurrent::False,
            };
            if current != AriaCurrent::False {
                node.set_aria_current(current);
            }
            node.set_children(children.remove(&source.id).unwrap_or_default());
            nodes.push((NodeId(source.id as u64), node));
        }

        let focus = presenter
            .access_focus()
            .map_or(ROOT, |id| NodeId(id as u64));
        Self { nodes, focus }
    }

    fn tree(&self) -> TreeUpdate {
        TreeUpdate {
            nodes: self.nodes.clone(),
            tree: Some(TreeInfo::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: self.focus,
        }
    }
}

fn map_node(source: &Source, origin: (f32, f32)) -> Node {
    let role = match source.role {
        role::TEXT => Role::Label,
        role::HEADING => Role::Heading,
        role::BUTTON | role::UP | role::TOGGLE => Role::Button,
        role::LINK => Role::Link,
        role::CHECKBOX => Role::CheckBox,
        role::SWITCH => Role::Switch,
        role::RADIO => Role::RadioButton,
        role::TAB => Role::Tab,
        role::TABLIST => Role::TabList,
        role::LIST => Role::List,
        role::LIST_ITEM => Role::ListItem,
        role::SCROLLER => Role::ScrollView,
        role::DROPDOWN => Role::ComboBox,
        role::LIST_BOX => Role::ListBox,
        role::OPTION => Role::ListBoxOption,
        _ => Role::GenericContainer,
    };
    let mut node = Node::new(role);
    let [x, y, width, height] = source.rect;
    let (x, y) = (f64::from(x + origin.0), f64::from(y + origin.1));
    node.set_bounds(Rect::new(x, y, x + f64::from(width), y + f64::from(height)));
    node.set_label(source.name.clone());
    if !source.hint.is_empty() {
        node.set_description(source.hint.clone());
    }
    match source.role {
        role::TEXT => node.set_value(source.name.clone()),
        role::DROPDOWN if !source.value.is_empty() => node.set_value(source.value.clone()),
        _ => {}
    }
    if source.flags & flag::DISABLED != 0 {
        node.set_disabled();
    }
    if source.flags & flag::CHECKABLE != 0 {
        node.set_toggled(if source.flags & flag::CHECKED != 0 {
            Toggled::True
        } else {
            Toggled::False
        });
    }
    let selected = source.flags & flag::SELECTED != 0;
    if selected || matches!(source.role, role::TAB | role::OPTION) {
        node.set_selected(selected);
    }
    if source.set.0 > 0 {
        node.set_position_in_set(source.set.0 as usize);
    }
    if source.set.1 > 0 {
        node.set_size_of_set(source.set.1 as usize);
    }
    if source.flags & flag::EXPANDABLE != 0 {
        node.set_expanded(source.flags & flag::EXPANDED != 0);
    }
    for kind in [
        ActionKind::Focus,
        ActionKind::Click,
        ActionKind::Expand,
        ActionKind::Collapse,
    ] {
        if kind.allowed(source) {
            node.add_action(kind.kit_action());
        }
    }
    node
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
enum ActionKind {
    Focus,
    Click,
    Expand,
    Collapse,
}

impl ActionKind {
    fn kit_action(self) -> Action {
        match self {
            Self::Focus => Action::Focus,
            Self::Click => Action::Click,
            Self::Expand => Action::Expand,
            Self::Collapse => Action::Collapse,
        }
    }

    fn allowed(self, source: &Source) -> bool {
        let flags = source.flags;
        flags & (flag::CLICK | flag::DISABLED) == flag::CLICK
            && match self {
                Self::Focus | Self::Click => true,
                Self::Expand => flags & (flag::EXPANDABLE | flag::EXPANDED) == flag::EXPANDABLE,
                Self::Collapse => {
                    flags & (flag::EXPANDABLE | flag::EXPANDED)
                        == (flag::EXPANDABLE | flag::EXPANDED)
                }
            }
    }
}

pub(crate) struct PostedAction {
    id: i32,
    kind: ActionKind,
}

impl PostedAction {
    /// If apply succeeds, transfer native focus outside the presenter's mutable borrow.
    pub(crate) fn wants_window_focus(&self) -> bool {
        self.kind == ActionKind::Focus
    }

    /// Run on the owner thread. Let the surface settle its own commands and repaint afterward.
    pub(crate) fn apply<D: DataSource>(&self, presenter: &mut Presenter<D>) -> bool {
        presenter.reader_access(1., false);
        if !presenter
            .access_told()
            .iter()
            .any(|node| node.id == self.id && self.kind.allowed(node))
        {
            return false;
        }
        let now = presenter.host().now();
        match self.kind {
            ActionKind::Focus => presenter.access_focus_node(self.id, now),
            _ => presenter.access_act(self.id, act::CLICK, 0., now),
        }
    }
}

struct Activate(Rc<Snapshot>);

impl ActivationHandler for Activate {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        Some(self.0.tree())
    }
}

#[derive(Clone, Copy)]
struct ActionPoster {
    hwnd: usize,
    message: u32,
    instance: usize,
}

impl ActionHandler for ActionPoster {
    fn do_action(&mut self, request: ActionRequest) {
        if request.target_tree != TreeId::ROOT || request.data.is_some() {
            return;
        }
        let Ok(id) = i32::try_from(request.target_node.0) else {
            return;
        };
        if id <= 0 {
            return;
        }
        let kind = match request.action {
            Action::Focus => ActionKind::Focus,
            Action::Click => ActionKind::Click,
            Action::Expand => ActionKind::Expand,
            Action::Collapse => ActionKind::Collapse,
            _ => return,
        };
        unsafe {
            PostMessageW(
                self.hwnd as HWND,
                self.message,
                self.instance | kind as usize,
                id as isize,
            )
        };
    }
}

pub(crate) struct Access {
    adapter: Adapter,
    snapshot: Rc<Snapshot>,
    poster: ActionPoster,
}

impl Access {
    /// Create the platform adapter on the window thread, outside window-state borrows.
    pub(crate) fn attach(hwnd: HWND, action_message: u32, snapshot: Snapshot) -> Self {
        let poster = ActionPoster {
            hwnd: hwnd as usize,
            message: action_message,
            instance: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed) << ACTION_BITS,
        };
        let adapter = Adapter::new(accesskit_windows::HWND(hwnd), false, poster);
        Self {
            adapter,
            snapshot: Rc::new(snapshot),
            poster,
        }
    }

    /// Convert the returned provider to LRESULT after releasing the surface borrow.
    pub(crate) fn object(
        &mut self,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<impl Into<accesskit_windows::LRESULT> + use<>> {
        self.adapter.handle_wm_getobject(
            accesskit_windows::WPARAM(wparam),
            accesskit_windows::LPARAM(lparam),
            &mut Activate(Rc::clone(&self.snapshot)),
        )
    }

    pub(crate) fn focus(&mut self, focused: bool) -> Option<QueuedEvents> {
        self.adapter.update_window_focus_state(focused)
    }

    /// Raise returned events after releasing window-state borrows because UIA can reenter.
    pub(crate) fn publish(&mut self, next: Snapshot) -> Option<QueuedEvents> {
        if *self.snapshot == next {
            return None;
        }
        self.snapshot = Rc::new(next);
        self.adapter.update_if_active(|| self.snapshot.tree())
    }

    /// Decode on the owner thread. The instance cookie rejects messages from a retired adapter.
    pub(crate) fn decode_action(
        &self,
        message: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> Option<PostedAction> {
        if message != self.poster.message || wp & !ACTION_MASK != self.poster.instance {
            return None;
        }
        let id = i32::try_from(lp).ok().filter(|&id| id > 0)?;
        let kind = match wp & ACTION_MASK {
            0 => ActionKind::Focus,
            1 => ActionKind::Click,
            2 => ActionKind::Expand,
            _ => ActionKind::Collapse,
        };
        let (_, node) = self
            .snapshot
            .nodes
            .iter()
            .find(|(key, _)| *key == NodeId(id as u64))?;
        (!node.is_disabled() && node.supports_action(kind.kit_action()))
            .then_some(PostedAction { id, kind })
    }
}

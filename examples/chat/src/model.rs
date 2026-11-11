//! Domain state and commands. Views read narrow graph attributes; commands batch
//! their writes through the host's action/update transaction.

use refract::prelude::*;
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversationId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Streaming,
    Complete,
    Stopped,
    Failed,
}

pub struct Message {
    pub id: MessageId,
    pub role: Role,
    pub body: State<String>,
    pub delivery: State<Delivery>,
}

#[derive(Clone)]
pub struct Request {
    pub id: RequestId,
    pub message: Rc<Message>,
    pub prompt: String,
}

pub struct Conversation {
    pub id: ConversationId,
    pub title: State<String>,
    pub draft: State<String>,
    pub messages: State<Vec<Rc<Message>>>,
    pub request: State<Option<Request>>,
    /// Equality filtering means typing after the first character need not rerun
    /// the Send button's view. The text control still receives the actual value.
    pub can_send: Derived<bool>,
    next_message: Cell<u64>,
    next_request: Cell<u64>,
}

impl Conversation {
    fn new(rt: &mut Runtime, id: ConversationId, title: &str) -> Rc<Self> {
        let draft = State::new(rt, String::new());
        let request = State::new(rt, None);
        let can_send = Derived::distinct(rt, {
            let draft = draft.clone();
            let request = request.clone();
            move |rt| request.get(rt).is_none() && !draft.get(rt).trim().is_empty()
        });
        Rc::new(Self {
            id,
            title: State::new(rt, title.into()),
            draft,
            messages: State::new(rt, vec![]),
            request,
            can_send,
            next_message: Cell::new(1),
            next_request: Cell::new(1),
        })
    }

    fn message(&self, rt: &mut Runtime, role: Role, body: &str, delivery: Delivery) -> Rc<Message> {
        let id = self.next_message.get();
        self.next_message.set(id + 1);
        Rc::new(Message {
            id: MessageId(id),
            role,
            body: State::new(rt, body.into()),
            delivery: State::new(rt, delivery),
        })
    }

    /// One request per conversation. Whitespace-only or repeated sends are no-ops.
    pub fn send(&self, rt: &mut Runtime) {
        if self.request.get(rt).is_some() {
            return;
        }
        let prompt = self.draft.get(rt).trim().to_owned();
        if prompt.is_empty() {
            return;
        }
        if self.messages.get(rt).is_empty() {
            let title: String = prompt
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(32)
                .collect();
            self.title.set(rt, title);
        }
        let user = self.message(rt, Role::User, &prompt, Delivery::Complete);
        let reply = self.message(rt, Role::Assistant, "", Delivery::Streaming);
        self.messages.update(rt, |messages| {
            messages.push(user);
            messages.push(reply.clone());
        });
        self.draft.set(rt, String::new());
        let id = self.next_request.get();
        self.next_request.set(id + 1);
        self.request.set(
            rt,
            Some(Request {
                id: RequestId(id),
                message: reply,
                prompt,
            }),
        );
    }

    pub fn suggest(&self, rt: &mut Runtime, prompt: &str) {
        if self.request.get(rt).is_none() {
            self.draft.set(rt, prompt.into());
            self.send(rt);
        }
    }

    /// Request identity rejects late callbacks from a stopped/replaced stream.
    /// Ordinary chunks touch exactly one attribute: the assistant message body.
    pub fn append(&self, rt: &mut Runtime, id: RequestId, chunk: &str, complete: bool) {
        let Some(request) = self.request.get(rt).clone().filter(|r| r.id == id) else {
            return;
        };
        if !chunk.is_empty() {
            request.message.body.update(rt, |body| body.push_str(chunk));
        }
        if complete {
            request.message.delivery.set(rt, Delivery::Complete);
            self.request.set(rt, None);
        }
    }

    pub fn fail(&self, rt: &mut Runtime, id: RequestId, error: String) {
        let Some(request) = self.request.get(rt).clone().filter(|r| r.id == id) else {
            return;
        };
        request.message.body.update(rt, |body| {
            if !body.is_empty() {
                body.push_str("\n\n");
            }
            body.push_str(&format!("Could not finish the reply: {error}"));
        });
        request.message.delivery.set(rt, Delivery::Failed);
        self.request.set(rt, None);
    }

    pub fn stop(&self, rt: &mut Runtime) {
        if let Some(request) = self.request.get(rt).clone() {
            request.message.delivery.set(rt, Delivery::Stopped);
            self.request.set(rt, None);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SidebarPreference {
    compact: bool,
    open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatFocus {
    Search,
    Composer(ConversationId),
}

pub struct ChatModel {
    pub focus: State<Option<ChatFocus>>,
    pub conversations: State<Vec<Rc<Conversation>>>,
    pub selected: State<ConversationId>,
    pub query: State<String>,
    sidebar_preference: State<Option<SidebarPreference>>,
    pub sidebar_visible: Derived<bool>,
    pub compact: Derived<bool>,
    pub navigation_error: State<Option<String>>,
    pub navigation_request: State<Option<refract::Request>>,
    surface: Option<Rc<Surface>>,
    next_conversation: Cell<u64>,
}

impl ChatModel {
    #[cfg(test)]
    pub fn new(rt: &mut Runtime) -> Self {
        Self::create(rt, None)
    }

    pub fn hosted(rt: &mut Runtime, surface: Rc<Surface>) -> Self {
        Self::create(rt, Some(surface))
    }

    fn create(rt: &mut Runtime, surface: Option<Rc<Surface>>) -> Self {
        let compact = Derived::distinct(rt, {
            let surface = surface.clone();
            move |rt| {
                surface
                    .as_ref()
                    .and_then(|surface| surface.viewport.get(rt))
                    .is_some_and(|viewport| viewport.width < 760.0)
            }
        });
        let sidebar_preference = State::new(rt, None::<SidebarPreference>);
        let sidebar_visible = Derived::distinct(rt, {
            let compact = compact.clone();
            let preference = sidebar_preference.clone();
            move |rt| {
                let compact = *compact.get(rt);
                preference
                    .get(rt)
                    .filter(|preference| preference.compact == compact)
                    .map_or(!compact, |preference| preference.open)
            }
        });
        let selected = surface
            .as_ref()
            .and_then(|surface| surface.location.get(rt))
            .as_ref()
            .and_then(conversation_id)
            .filter(|id| (1..=3).contains(&id.0))
            .unwrap_or(ConversationId(1));
        let first = Conversation::new(rt, ConversationId(1), "A calmer kind of workspace");
        let user = first.message(
            rt,
            Role::User,
            "What makes a workspace feel calm and focused?",
            Delivery::Complete,
        );
        let assistant = first.message(rt, Role::Assistant,
            "A calm workspace gives your attention somewhere to land. A few deliberate choices can make a real difference.\n\nMake room for the work\nKeep the current task in view and let everything else recede.\n\nFind a comfortable rhythm\nPair a focused stretch with a real break. Come back with fresh eyes.\n\nLeave a little personality\nA favorite notebook or a warm light can make a useful space feel like your own.", Delivery::Complete);
        first.messages.set(rt, vec![user, assistant]);
        let second = Conversation::new(rt, ConversationId(2), "Designing with intention");
        let third = Conversation::new(rt, ConversationId(3), "A weekend away");
        Self {
            focus: State::new(rt, None),
            conversations: State::new(rt, vec![first, second, third]),
            selected: State::new(rt, selected),
            query: State::new(rt, String::new()),
            sidebar_preference,
            sidebar_visible,
            compact,
            surface,
            navigation_error: State::new(rt, None),
            navigation_request: State::new(rt, None),
            next_conversation: Cell::new(4),
        }
    }

    pub fn toggle_sidebar(&self, rt: &mut Runtime) {
        let preference = SidebarPreference {
            compact: *self.compact.get(rt),
            open: !*self.sidebar_visible.get(rt),
        };
        self.sidebar_preference.set(rt, Some(preference));
    }

    fn close_compact_sidebar(&self, rt: &mut Runtime) {
        if *self.compact.get(rt) {
            self.sidebar_preference.set_if_changed(
                rt,
                Some(SidebarPreference {
                    compact: true,
                    open: false,
                }),
            );
        }
    }

    fn navigate(&self, rt: &mut Runtime, id: ConversationId, mode: HistoryMode) {
        let Some(surface) = &self.surface else {
            return;
        };
        let Some(mut url) = surface.location.get(rt) else {
            return;
        };
        // Query routing preserves deployment prefixes and works on static hosts.
        let pairs: Vec<_> = url
            .query_pairs()
            .filter(|(key, _)| key != "conversation")
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        url.set_query(None);
        url.query_pairs_mut()
            .extend_pairs(pairs)
            .append_pair("conversation", &id.0.to_string());
        match surface.navigate(rt, url.as_str(), mode) {
            Ok(request) => {
                self.navigation_error.set_if_changed(rt, None);
                self.navigation_request.set(rt, Some(request));
            }
            Err(error) => self.navigation_error.set(rt, Some(error.to_string())),
        }
    }

    /// Host navigation changes selection without pushing another history entry.
    pub fn apply_location(&self, rt: &mut Runtime, location: Option<Url>) {
        let Some(url) = location else {
            return;
        };
        if let Some(id) = conversation_id(&url)
            .filter(|id| self.conversations.get(rt).iter().any(|chat| chat.id == *id))
        {
            self.selected.set_if_changed(rt, id);
            self.close_compact_sidebar(rt);
        } else {
            // Deleted or session-only conversations cannot be restored after a reload.
            self.navigate(rt, *self.selected.get(rt), HistoryMode::Replace);
        }
    }

    pub fn new_chat(&self, rt: &mut Runtime) {
        self.create_chat(rt, HistoryMode::Push);
        self.focus
            .set(rt, Some(ChatFocus::Composer(*self.selected.get(rt))));
    }

    fn create_chat(&self, rt: &mut Runtime, mode: HistoryMode) {
        let id = self.next_conversation.get();
        self.next_conversation.set(id + 1);
        let chat = Conversation::new(rt, ConversationId(id), "New conversation");
        self.conversations.update(rt, |items| items.insert(0, chat));
        self.query.set_if_changed(rt, String::new());
        self.selected.set(rt, ConversationId(id));
        self.close_compact_sidebar(rt);
        self.navigate(rt, ConversationId(id), mode);
    }

    pub fn select(&self, rt: &mut Runtime, id: ConversationId) {
        if self.conversations.get(rt).iter().any(|c| c.id == id) {
            let changed = *self.selected.get(rt) != id;
            self.selected.set_if_changed(rt, id);
            self.close_compact_sidebar(rt);
            if changed {
                self.navigate(rt, id, HistoryMode::Push);
            }
        }
    }

    pub fn delete(&self, rt: &mut Runtime, id: ConversationId) {
        let Some(chat) = self
            .conversations
            .get(rt)
            .iter()
            .find(|c| c.id == id)
            .cloned()
        else {
            return;
        };
        chat.stop(rt);
        self.conversations
            .update(rt, |items| items.retain(|c| c.id != id));
        if self.conversations.get(rt).is_empty() {
            self.create_chat(rt, HistoryMode::Replace);
        } else if *self.selected.get(rt) == id {
            let first = self.conversations.get(rt)[0].id;
            self.selected.set(rt, first);
            self.navigate(rt, first, HistoryMode::Replace);
        }
    }
}

fn conversation_id(url: &Url) -> Option<ConversationId> {
    url.query_pairs()
        .find(|(key, _)| key == "conversation")?
        .1
        .parse()
        .ok()
        .map(ConversationId)
}

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

    pub fn stop(&self, rt: &mut Runtime) {
        if let Some(request) = self.request.get(rt).clone() {
            request.message.delivery.set(rt, Delivery::Stopped);
            self.request.set(rt, None);
        }
    }
}

pub struct ChatModel {
    pub conversations: State<Vec<Rc<Conversation>>>,
    pub selected: State<ConversationId>,
    pub query: State<String>,
    pub sidebar_open: State<bool>,
    next_conversation: Cell<u64>,
}

impl ChatModel {
    pub fn new(rt: &mut Runtime) -> Self {
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
            conversations: State::new(rt, vec![first, second, third]),
            selected: State::new(rt, ConversationId(1)),
            query: State::new(rt, String::new()),
            sidebar_open: State::new(rt, true),
            next_conversation: Cell::new(4),
        }
    }

    pub fn new_chat(&self, rt: &mut Runtime) {
        let id = self.next_conversation.get();
        self.next_conversation.set(id + 1);
        let chat = Conversation::new(rt, ConversationId(id), "New conversation");
        self.conversations.update(rt, |items| items.insert(0, chat));
        self.query.set_if_changed(rt, String::new());
        self.selected.set(rt, ConversationId(id));
    }

    pub fn select(&self, rt: &mut Runtime, id: ConversationId) {
        if self.conversations.get(rt).iter().any(|c| c.id == id) {
            self.selected.set_if_changed(rt, id);
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
            self.new_chat(rt);
        } else if *self.selected.get(rt) == id {
            let first = self.conversations.get(rt)[0].id;
            self.selected.set(rt, first);
        }
    }
}

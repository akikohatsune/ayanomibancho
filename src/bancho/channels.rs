#![allow(dead_code)]

use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct Channel {
    pub name: String,
    pub topic: String,
    pub auto_join: bool,
    pub read_only: bool,
    pub members: HashSet<i32>,
}

impl Channel {
    pub fn new(name: &str, topic: &str, auto_join: bool, read_only: bool) -> Self {
        Self {
            name: name.to_string(),
            topic: topic.to_string(),
            auto_join,
            read_only,
            members: HashSet::new(),
        }
    }

    pub fn add_member(&mut self, user_id: i32) {
        self.members.insert(user_id);
    }

    pub fn remove_member(&mut self, user_id: i32) {
        self.members.remove(&user_id);
    }

    pub fn has_member(&self, user_id: i32) -> bool {
        self.members.contains(&user_id)
    }

    pub fn user_count(&self) -> i16 {
        self.members.len() as i16
    }
}

pub fn default_channels() -> Vec<Channel> {
    vec![
        Channel::new("#osu", "General discussion channel", true, false),
        Channel::new("#announce", "Server announcements", true, true),
        Channel::new("#lobby", "Multiplayer lobby chat", false, false),
        Channel::new("#multiplayer", "Multiplayer match channel", false, false),
        Channel::new("#spectator", "Spectator chat channel", false, false),
    ]
}

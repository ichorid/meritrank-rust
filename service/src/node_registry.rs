use std::collections::HashMap;


#[derive(Debug, Clone)]
pub struct NodeInfo {
    id: usize,
    kind: NodeKind,
}

#[derive(Clone)]
pub struct NodeRegistry {
    name_to_info: HashMap<NodeName, NodeInfo>,
    id_to_name: Vec<NodeName>,
    next_id: usize,
}

impl NodeRegistry {
    pub fn new() -> Self {
        NodeRegistry {
            name_to_info: HashMap::new(),
            id_to_name: Vec::new(),
            next_id: 0,
        }
    }

    pub fn register(&mut self, name: NodeName, kind: NodeKind) -> usize {
        if let Some(info) = self.name_to_info.get(&name) {
            return info.id;
        }

        let id = self.next_id;
        self.next_id += 1;

        let info = NodeInfo { id, kind };
        self.name_to_info.insert(name.clone(), info);
        self.id_to_name.push(name);

        id
    }

    pub fn get_id(&self, name: &str) -> Option<usize> {
        self.name_to_info.get(name).map(|info| info.id)
    }

    pub fn get_name(&self, id: usize) -> Option<&str> {
        self.id_to_name.get(id).map(|s| s.as_str())
    }

    pub fn get_kind(&self, name: &str) -> Option<NodeKind> {
        self.name_to_info.get(name).map(|info| info.kind)
    }

    pub fn get_kind_by_id(&self, id: usize) -> Option<NodeKind> {
        self.get_name(id).and_then(|name| self.get_kind(name))
    }

    pub fn len(&self) -> usize {
        self.id_to_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.id_to_name.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_registry() {
        let mut registry = NodeRegistry::new();

        let user_id = registry.register("Alice".to_string(), NodeKind::User);
        assert_eq!(user_id, 0);
        assert_eq!(registry.get_id("Alice"), Some(0));
        assert_eq!(registry.get_name(0), Some("Alice"));
        assert_eq!(registry.get_kind("Alice"), Some(NodeKind::User));

        let comment_id = registry.register("Comment1".to_string(), NodeKind::Comment);
        assert_eq!(comment_id, 1);
        assert_eq!(registry.get_id("Comment1"), Some(1));
        assert_eq!(registry.get_name(1), Some("Comment1"));
        assert_eq!(registry.get_kind("Comment1"), Some(NodeKind::Comment));

        // Test registering an existing name
        let existing_id = registry.register("Alice".to_string(), NodeKind::User);
        assert_eq!(existing_id, 0);

        assert_eq!(registry.len(), 2);
        assert!(!registry.is_empty());

        // Test non-existent entries
        assert_eq!(registry.get_id("Bob"), None);
        assert_eq!(registry.get_name(2), None);
        assert_eq!(registry.get_kind("Bob"), None);
        assert_eq!(registry.get_kind_by_id(2), None);
    }
}
use crate::skill::Skill;

pub struct DynamicSkillManager {
    loaded_skills: std::collections::HashMap<String, Box<dyn Skill + Send + Sync>>,
}

impl Default for DynamicSkillManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DynamicSkillManager {
    pub fn new() -> Self {
        Self {
            loaded_skills: std::collections::HashMap::new(),
        }
    }

    pub fn install_skill(&mut self, name: &str, skill: Box<dyn Skill + Send + Sync>) {
        self.loaded_skills.insert(name.to_string(), skill);
    }

    pub fn uninstall_skill(&mut self, name: &str) -> bool {
        self.loaded_skills.remove(name).is_some()
    }

    pub fn list_skills(&self) -> Vec<String> {
        self.loaded_skills.keys().cloned().collect()
    }
}

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum CVarValue {
    Int(i32),
    Float(f32),
    Bool(bool),
    String(String),
}

impl std::fmt::Display for CVarValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CVarValue::Int(v) => write!(f, "{}", v),
            CVarValue::Float(v) => write!(f, "{}", v),
            CVarValue::Bool(v) => write!(f, "{}", v),
            CVarValue::String(v) => write!(f, "{}", v),
        }
    }
}

pub struct CVar {
    pub name: String,
    pub description: String,
    pub value: CVarValue,
    pub default_value: CVarValue,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct DevConsoleState {
    pub is_open: bool,
    pub input_buffer: String,
    pub output_log: Vec<String>,
}

pub struct CVarRegistry {
    pub cvars: HashMap<String, CVar>,
    pub command_history: Vec<String>,
}

impl CVarRegistry {
    pub fn new() -> Self {
        Self {
            cvars: HashMap::new(),
            command_history: Vec::new(),
        }
    }

    pub fn register(&mut self, name: &str, description: &str, value: CVarValue) {
        self.cvars.insert(name.to_lowercase(), CVar {
            name: name.to_string(),
            description: description.to_string(),
            default_value: value.clone(),
            value,
        });
    }

    pub fn get(&self, name: &str) -> Option<&CVarValue> {
        self.cvars.get(&name.to_lowercase()).map(|c| &c.value)
    }

    pub fn set(&mut self, name: &str, value: CVarValue) -> Result<(), String> {
        if let Some(cvar) = self.cvars.get_mut(&name.to_lowercase()) {
            cvar.value = value;
            Ok(())
        } else {
            Err(format!("CVar '{}' bulunamadi.", name))
        }
    }
    
    // Command parser (e.g., "set physics_gravity_y -10.5")
    pub fn execute(&mut self, cmd: &str) -> String {
        self.command_history.push(cmd.to_string());
        let parts: Vec<&str> = cmd.trim().split_whitespace().collect();
        if parts.is_empty() { return String::new(); }

        let command = parts[0].to_lowercase();
        match command.as_str() {
            "set" => {
                if parts.len() < 3 {
                    return "Kullanim: set <cvar> <value>".to_string();
                }
                let cvar_name = parts[1].to_lowercase();
                let value_str = parts[2..].join(" ");
                
                if let Some(cvar) = self.cvars.get_mut(&cvar_name) {
                    // Try to parse based on existing type
                    match &cvar.default_value {
                        CVarValue::Int(_) => {
                            if let Ok(v) = value_str.parse::<i32>() {
                                cvar.value = CVarValue::Int(v);
                                format!("{} = {}", cvar_name, v)
                            } else {
                                "Hata: Beklenen tip Int".to_string()
                            }
                        },
                        CVarValue::Float(_) => {
                            if let Ok(v) = value_str.parse::<f32>() {
                                cvar.value = CVarValue::Float(v);
                                format!("{} = {}", cvar_name, v)
                            } else {
                                "Hata: Beklenen tip Float".to_string()
                            }
                        },
                        CVarValue::Bool(_) => {
                            if let Ok(v) = value_str.parse::<bool>() {
                                cvar.value = CVarValue::Bool(v);
                                format!("{} = {}", cvar_name, v)
                            } else if value_str == "1" {
                                cvar.value = CVarValue::Bool(true);
                                format!("{} = true", cvar_name)
                            } else if value_str == "0" {
                                cvar.value = CVarValue::Bool(false);
                                format!("{} = false", cvar_name)
                            } else {
                                "Hata: Beklenen tip Bool".to_string()
                            }
                        },
                        CVarValue::String(_) => {
                            cvar.value = CVarValue::String(value_str.clone());
                            format!("{} = \"{}\"", cvar_name, value_str)
                        }
                    }
                } else {
                    format!("Bilinmeyen cvar: {}", cvar_name)
                }
            },
            "get" => {
                if parts.len() < 2 {
                    return "Kullanim: get <cvar>".to_string();
                }
                let cvar_name = parts[1].to_lowercase();
                if let Some(cvar) = self.cvars.get(&cvar_name) {
                    format!("{} = {}", cvar.name, cvar.value)
                } else {
                    format!("Bilinmeyen cvar: {}", cvar_name)
                }
            },
            "list" => {
                let mut out = String::from("Kayitli CVar'lar:\n");
                for (name, cvar) in &self.cvars {
                    out.push_str(&format!("  {} = {} ({})\n", name, cvar.value, cvar.description));
                }
                out
            },
            "clear" => {
                String::from("CLEAR_SCREEN_REQUEST") // Special signal
            },
            _ => {
                format!("Bilinmeyen komut: {}. Mevcut komutlar: set, get, list, clear", command)
            }
        }
    }
}

impl Default for CVarRegistry {
    fn default() -> Self {
        Self::new()
    }
}

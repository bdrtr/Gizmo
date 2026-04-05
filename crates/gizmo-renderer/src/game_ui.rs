/// Gizmo Oyun İçi UI (Game UI) — Runtime buton, slider, text, health bar vb.
/// Egui'den farklı olarak bu UI sistemi oyun ekranının bir parçasıdır.
/// Anchor (çapa) sistemi ile ekran kenarlarına otomatik hizalanır.

/// Ekrandaki UI elemanının konumlandırma çapası
#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

/// UI Elemanı — Her eleman bir dikdörtgen alan kaplar
#[derive(Clone, Debug)]
pub struct UiElement {
    pub id: String,
    pub kind: UiKind,
    pub anchor: Anchor,
    pub offset: [f32; 2],    // Çapa noktasından piksel offset
    pub size: [f32; 2],      // Genişlik, yükseklik (piksel)
    pub visible: bool,
    pub color: [f32; 4],     // RGBA renk
}

/// UI Eleman türü
#[derive(Clone, Debug)]
pub enum UiKind {
    /// Yazı metni
    Text {
        content: String,
        font_size: f32,
    },
    /// Tıklanabilir buton
    Button {
        label: String,
        pressed: bool,
        hovered: bool,
    },
    /// Sürgülü değer (min-max arası)
    Slider {
        value: f32,
        min: f32,
        max: f32,
        label: String,
    },
    /// İlerleme çubuğu (can barı, yükleme vb.)
    ProgressBar {
        value: f32,      // 0.0 - 1.0 arası
        fill_color: [f32; 4],
        label: Option<String>,
    },
    /// Resim/İkon
    Image {
        texture_source: String,
    },
}

/// Oyun İçi UI Canvas — Tüm UI elemanlarını yöneten yapı
pub struct UiCanvas {
    pub elements: Vec<UiElement>,
}

impl UiCanvas {
    pub fn new() -> Self {
        Self { elements: Vec::new() }
    }
}

impl Default for UiCanvas {
    fn default() -> Self {
        Self::new()
    }
}

impl UiCanvas {

    /// Yeni bir text elemanı ekler
    pub fn add_text(&mut self, id: &str, content: &str, anchor: Anchor, offset: [f32; 2], font_size: f32, color: [f32; 4]) -> &mut Self {
        self.elements.push(UiElement {
            id: id.to_string(),
            kind: UiKind::Text { content: content.to_string(), font_size },
            anchor, offset,
            size: [0.0, 0.0], // Auto-size
            visible: true,
            color,
        });
        self
    }

    /// Yeni bir buton ekler
    pub fn add_button(&mut self, id: &str, label: &str, anchor: Anchor, offset: [f32; 2], size: [f32; 2], color: [f32; 4]) -> &mut Self {
        self.elements.push(UiElement {
            id: id.to_string(),
            kind: UiKind::Button { label: label.to_string(), pressed: false, hovered: false },
            anchor, offset, size,
            visible: true,
            color,
        });
        self
    }

    /// Yeni bir slider ekler
    pub fn add_slider(&mut self, id: &str, label: &str, value: f32, min: f32, max: f32, anchor: Anchor, offset: [f32; 2], size: [f32; 2]) -> &mut Self {
        self.elements.push(UiElement {
            id: id.to_string(),
            kind: UiKind::Slider { value, min, max, label: label.to_string() },
            anchor, offset, size,
            visible: true,
            color: [0.3, 0.3, 0.3, 1.0],
        });
        self
    }

    /// Yeni bir progress bar ekler
    pub fn add_progress_bar(&mut self, id: &str, value: f32, fill_color: [f32; 4], anchor: Anchor, offset: [f32; 2], size: [f32; 2]) -> &mut Self {
        self.elements.push(UiElement {
            id: id.to_string(),
            kind: UiKind::ProgressBar { value, fill_color, label: None },
            anchor, offset, size,
            visible: true,
            color: [0.2, 0.2, 0.2, 0.8],
        });
        self
    }

    /// ID ile bir UI elemanını bul ve referansını döndür
    pub fn get(&self, id: &str) -> Option<&UiElement> {
        self.elements.iter().find(|e| e.id == id)
    }

    /// ID ile bir UI elemanını bul ve mutable referans döndür
    pub fn get_mut(&mut self, id: &str) -> Option<&mut UiElement> {
        self.elements.iter_mut().find(|e| e.id == id)
    }

    /// Butonun basılı olup olmadığını kontrol et
    pub fn is_button_pressed(&self, id: &str) -> bool {
        if let Some(el) = self.get(id) {
            if let UiKind::Button { pressed, .. } = &el.kind {
                return *pressed;
            }
        }
        false
    }

    /// Slider değerini al
    pub fn get_slider_value(&self, id: &str) -> Option<f32> {
        if let Some(el) = self.get(id) {
            if let UiKind::Slider { value, .. } = &el.kind {
                return Some(*value);
            }
        }
        None
    }

    /// Progress bar değerini güncelle
    pub fn set_progress(&mut self, id: &str, value: f32) {
        if let Some(el) = self.get_mut(id) {
            if let UiKind::ProgressBar { value: ref mut v, .. } = &mut el.kind {
                *v = value.clamp(0.0, 1.0);
            }
        }
    }

    /// Text içeriğini güncelle
    pub fn set_text(&mut self, id: &str, content: &str) {
        if let Some(el) = self.get_mut(id) {
            if let UiKind::Text { content: ref mut c, .. } = &mut el.kind {
                *c = content.to_string();
            }
        }
    }

    /// Elemanın ekrandaki piksel pozisyonunu hesapla (Anchor + Offset)
    pub fn compute_position(&self, element: &UiElement, screen_w: f32, screen_h: f32) -> [f32; 2] {
        let (ax, ay) = match element.anchor {
            Anchor::TopLeft      => (0.0, 0.0),
            Anchor::TopCenter    => (screen_w / 2.0, 0.0),
            Anchor::TopRight     => (screen_w, 0.0),
            Anchor::CenterLeft   => (0.0, screen_h / 2.0),
            Anchor::Center       => (screen_w / 2.0, screen_h / 2.0),
            Anchor::CenterRight  => (screen_w, screen_h / 2.0),
            Anchor::BottomLeft   => (0.0, screen_h),
            Anchor::BottomCenter => (screen_w / 2.0, screen_h),
            Anchor::BottomRight  => (screen_w, screen_h),
        };
        [ax + element.offset[0], ay + element.offset[1]]
    }

    /// Mouse ile etkileşim kontrolü (buton hover, press, slider drag)
    pub fn handle_input(&mut self, mouse_x: f32, mouse_y: f32, mouse_pressed: bool, screen_w: f32, screen_h: f32) {
        for element in &mut self.elements {
            if !element.visible { continue; }
            
            let pos = {
                let (ax, ay) = match element.anchor {
                    Anchor::TopLeft      => (0.0, 0.0),
                    Anchor::TopCenter    => (screen_w / 2.0, 0.0),
                    Anchor::TopRight     => (screen_w, 0.0),
                    Anchor::CenterLeft   => (0.0, screen_h / 2.0),
                    Anchor::Center       => (screen_w / 2.0, screen_h / 2.0),
                    Anchor::CenterRight  => (screen_w, screen_h / 2.0),
                    Anchor::BottomLeft   => (0.0, screen_h),
                    Anchor::BottomCenter => (screen_w / 2.0, screen_h),
                    Anchor::BottomRight  => (screen_w, screen_h),
                };
                [ax + element.offset[0], ay + element.offset[1]]
            };

            let in_bounds = mouse_x >= pos[0] && mouse_x <= pos[0] + element.size[0]
                         && mouse_y >= pos[1] && mouse_y <= pos[1] + element.size[1];
            
            match &mut element.kind {
                UiKind::Button { pressed, hovered, .. } => {
                    *hovered = in_bounds;
                    *pressed = in_bounds && mouse_pressed;
                }
                UiKind::Slider { value, min, max, .. } => {
                    if in_bounds && mouse_pressed && element.size[0] > 0.0 {
                        let local_x = mouse_x - pos[0];
                        let ratio = (local_x / element.size[0]).clamp(0.0, 1.0);
                        *value = *min + ratio * (*max - *min);
                    }
                }
                _ => {}
            }
        }
    }
}

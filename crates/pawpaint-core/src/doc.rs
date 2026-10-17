//! O documento: tamanho, camadas de pixels e a seleção.

use crate::blend::BlendMode;
use crate::selection::Selection;
use crate::tile::TiledImage;

pub type LayerId = u64;

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    /// Travada: não aceita pintura nem filtros.
    pub locked: bool,
    /// Trava de alfa: só dá para mudar a cor de onde já tem tinta.
    pub alpha_lock: bool,
    pub opacity: f32,
    pub mode: BlendMode,
    pub pixels: TiledImage,
    /// Máscara da camada: o alfa de cada pixel é o quanto dela fica escondido (vazia = tudo visível).
    pub mask: Option<TiledImage>,
    /// Pasta: não tem pixels, só junta as camadas que têm esta como `parent`.
    pub is_group: bool,
    /// Pasta onde a camada está (`None` = no topo).
    pub parent: Option<LayerId>,
    /// Só para pastas: mostrando as camadas de dentro na lista.
    pub expanded: bool,
    /// Camada de ajuste: não tem pixels; aplica este filtro a tudo que está embaixo dela
    /// (na mesma pasta), na força da opacidade e limitado pela máscara.
    pub adjust: Option<crate::filters::Filter>,
}

impl Layer {
    pub fn new(id: LayerId, name: impl Into<String>, w: usize, h: usize) -> Self {
        Self {
            id,
            name: name.into(),
            visible: true,
            locked: false,
            alpha_lock: false,
            opacity: 1.0,
            mode: BlendMode::Normal,
            pixels: TiledImage::new(w, h),
            mask: None,
            is_group: false,
            parent: None,
            expanded: true,
            adjust: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    pub width: usize,
    pub height: usize,
    /// De baixo para cima.
    pub layers: Vec<Layer>,
    /// `None` = nada selecionado (tudo editável).
    pub selection: Option<Selection>,
    pub(crate) next_id: LayerId,
}

impl Doc {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            layers: Vec::new(),
            selection: None,
            next_id: 1,
        }
    }

    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    pub fn index_of(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    pub(crate) fn fresh(&mut self) -> LayerId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn next_id(&self) -> LayerId {
        self.next_id
    }

    pub fn set_next_id(&mut self, id: LayerId) {
        self.next_id = id.max(self.layers.iter().map(|l| l.id + 1).max().unwrap_or(1));
    }
}

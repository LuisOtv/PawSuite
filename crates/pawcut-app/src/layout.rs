//! Layout dos painéis: esconder e mostrar um painel devolve ele ao lugar (e tamanho) de antes.

use std::collections::{BTreeSet, HashMap};

use egui_dock::{DockState, NodeIndex};

use crate::panels::Panel;

/// Monta o layout da página Edit do Resolve só com os painéis de `visible`:
/// Media Pool/Effects à esquerda, Source e Timeline Viewer no meio, Inspector à direita,
/// Timeline embaixo.
pub fn resolve_layout(visible: &BTreeSet<Panel>) -> DockState<Panel> {
    let has = |p: Panel| visible.contains(&p);
    // Colunas de cima, da esquerda para a direita, com a largura relativa de cada uma.
    let mut columns: Vec<(Vec<Panel>, f32)> = Vec::new();
    let left: Vec<Panel> = [Panel::MediaPool, Panel::Effects, Panel::Notes]
        .into_iter()
        .filter(|p| has(*p))
        .collect();
    if !left.is_empty() {
        columns.push((left, 0.21));
    }
    for (p, w) in [
        (Panel::SourceViewer, 0.29),
        (Panel::TimelineViewer, 0.29),
        (Panel::Inspector, 0.21),
    ] {
        if has(p) {
            columns.push((vec![p], w));
        }
    }
    let timeline = has(Panel::Timeline);
    let first = match columns.first() {
        Some((c, _)) => c[0],
        None => Panel::Timeline,
    };
    let mut dock = DockState::new(vec![first]);
    if columns.is_empty() {
        return dock;
    }
    let tree = dock.main_surface_mut();
    let mut top = NodeIndex::root();
    if timeline {
        let [t, _] = tree.split_below(NodeIndex::root(), 0.56, vec![Panel::Timeline]);
        top = t;
    }
    // Divide da esquerda para a direita: o nó da esquerda fica com a fração dele no que sobra.
    let total: f32 = columns.iter().map(|c| c.1).sum();
    let mut remaining = total;
    let mut current = top;
    let mut nodes = Vec::new();
    for (i, (_, w)) in columns.iter().enumerate() {
        if i + 1 == columns.len() {
            nodes.push(current);
            break;
        }
        let next = columns[i + 1].0[0];
        let [left_node, right_node] = tree.split_right(current, w / remaining, vec![next]);
        nodes.push(left_node);
        remaining -= w;
        current = right_node;
    }
    // Effects (e Notas) embaixo do Media Pool, na mesma coluna.
    if let Some((first_col, _)) = columns.first()
        && first_col.len() >= 2
    {
        tree.split_below(nodes[0], 0.62, first_col[1..].to_vec());
    }
    dock
}

pub struct PanelLayout {
    pub dock: DockState<Panel>,
    /// Último layout usado com cada combinação de painéis visíveis.
    saved: HashMap<BTreeSet<Panel>, DockState<Panel>>,
    /// Layout de antes de maximizar um painel (` volta para ele).
    pub before_maximize: Option<DockState<Panel>>,
}

impl PanelLayout {
    pub fn new(dock: DockState<Panel>) -> Self {
        Self {
            dock,
            saved: HashMap::new(),
            before_maximize: None,
        }
    }

    /// Como o Resolve em "single viewer": sem o painel Source (o Timeline Viewer mostra a
    /// mídia sob o mouse no Media Pool e vira Source ao clicar numa mídia).
    pub fn default_layout() -> DockState<Panel> {
        resolve_layout(
            &Panel::ALL
                .into_iter()
                .filter(|p| !matches!(p, Panel::SourceViewer | Panel::Notes))
                .collect(),
        )
    }

    pub fn visible(&self) -> BTreeSet<Panel> {
        Panel::ALL
            .into_iter()
            .filter(|p| self.dock.find_tab(p).is_some())
            .collect()
    }

    pub fn is_visible(&self, panel: Panel) -> bool {
        self.dock.find_tab(&panel).is_some()
    }

    /// Esconde ou mostra `panel`. Se essa combinação de painéis já foi usada, volta exatamente
    /// como estava; senão monta no lugar do Resolve.
    /// Maximiza `panel` (só ele na janela) ou volta ao layout de antes.
    pub fn toggle_maximize(&mut self, panel: Panel) {
        match self.before_maximize.take() {
            Some(d) => self.dock = d,
            None => {
                self.before_maximize = Some(self.dock.clone());
                self.dock = DockState::new(vec![panel]);
            }
        }
    }

    /// O layout "de verdade" (o de antes de maximizar, se estiver maximizado).
    pub fn persistent(&self) -> &DockState<Panel> {
        self.before_maximize.as_ref().unwrap_or(&self.dock)
    }

    pub fn toggle(&mut self, panel: Panel) {
        if let Some(d) = self.before_maximize.take() {
            self.dock = d;
        }
        let now = self.visible();
        let mut next = now.clone();
        if !next.remove(&panel) {
            next.insert(panel);
        }
        if next.is_empty() {
            return;
        }
        self.saved.insert(now, self.dock.clone());
        self.dock = match self.saved.get(&next) {
            Some(d) => d.clone(),
            None => resolve_layout(&next),
        };
    }

    /// Garante que `panel` está aberto e com a aba na frente.
    pub fn reveal(&mut self, panel: Panel) {
        if !self.is_visible(panel) {
            self.toggle(panel);
        }
        if let Some(loc) = self.dock.find_tab(&panel) {
            let _ = self.dock.set_active_tab(loc);
        }
    }

    pub fn reset(&mut self) {
        self.saved.clear();
        self.before_maximize = None;
        self.dock = Self::default_layout();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(d: &DockState<Panel>) -> Vec<Panel> {
        let mut v: Vec<Panel> = d.iter_all_tabs().map(|(_, t)| *t).collect();
        v.sort();
        v
    }

    #[test]
    fn every_subset_builds_with_each_panel_once() {
        for mask in 1u32..(1 << Panel::ALL.len()) {
            let set: BTreeSet<Panel> = Panel::ALL
                .into_iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, p)| p)
                .collect();
            let d = resolve_layout(&set);
            assert_eq!(
                tabs(&d),
                set.iter().copied().collect::<Vec<_>>(),
                "conjunto {set:?}"
            );
        }
    }

    #[test]
    fn hiding_and_showing_restores_the_exact_layout() {
        let mut l = PanelLayout::new(resolve_layout(&Panel::ALL.into_iter().collect()));
        let before = serde_json::to_string(&l.dock).unwrap();
        l.toggle(Panel::MediaPool);
        assert!(!l.is_visible(Panel::MediaPool));
        assert!(l.is_visible(Panel::Effects));
        l.toggle(Panel::Inspector);
        l.toggle(Panel::Inspector);
        l.toggle(Panel::MediaPool);
        assert_eq!(serde_json::to_string(&l.dock).unwrap(), before);
    }

    #[test]
    fn a_hidden_panel_comes_back_in_its_column_not_full_width() {
        let mut l = PanelLayout::new(PanelLayout::default_layout());
        l.toggle(Panel::Effects);
        l.toggle(Panel::MediaPool);
        // nenhum layout guardado para "Effects sozinho na esquerda": monta no lugar do Resolve
        l.toggle(Panel::Effects);
        let expected = resolve_layout(&l.visible());
        assert_eq!(
            serde_json::to_string(&l.dock).unwrap(),
            serde_json::to_string(&expected).unwrap()
        );
        assert!(l.is_visible(Panel::Timeline) && l.is_visible(Panel::Effects));
    }

    #[test]
    fn the_last_panel_cannot_be_hidden() {
        let mut l = PanelLayout::new(resolve_layout(&[Panel::Timeline].into_iter().collect()));
        l.toggle(Panel::Timeline);
        assert!(l.is_visible(Panel::Timeline));
    }

    #[test]
    fn maximize_shows_one_panel_and_restores() {
        let mut l = PanelLayout::new(PanelLayout::default_layout());
        let before = l.visible();
        l.toggle_maximize(Panel::Timeline);
        assert_eq!(l.visible().len(), 1);
        assert!(l.is_visible(Panel::Timeline));
        assert_eq!(l.persistent().iter_all_tabs().count(), before.len());
        l.toggle_maximize(Panel::Timeline);
        assert_eq!(l.visible(), before);
    }
}

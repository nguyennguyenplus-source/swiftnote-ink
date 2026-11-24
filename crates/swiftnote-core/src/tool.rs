use crate::brush::{default_pen_slots, BrushStyle};
use crate::eraser::EraserMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTool {
    Pen(usize),
    Eraser,
    Select,
    Laser,
}

#[derive(Debug, Clone)]
pub struct ToolState {
    pub pen_slots: [BrushStyle; 5],
    pub active_pen_slot: usize,
    pub active_tool: ActiveTool,
    pub eraser_mode: EraserMode,
    pub back_to_pen_after_erase: bool,
    hardware_eraser_prev_tool: Option<ActiveTool>,
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            pen_slots: default_pen_slots(),
            active_pen_slot: 0,
            active_tool: ActiveTool::Pen(0),
            eraser_mode: EraserMode::Stroke,
            back_to_pen_after_erase: false,
            hardware_eraser_prev_tool: None,
        }
    }
}

impl ToolState {
    pub fn select_pen_slot(&mut self, idx: usize) {
        let clamped = idx.min(self.pen_slots.len() - 1);
        self.active_pen_slot = clamped;
        self.active_tool = ActiveTool::Pen(clamped);
    }

    pub fn select_eraser(&mut self) {
        self.active_tool = ActiveTool::Eraser;
    }

    #[inline]
    pub fn active_brush(&self) -> BrushStyle {
        self.pen_slots[self.active_pen_slot]
    }

    #[inline]
    pub fn active_brush_mut(&mut self) -> &mut BrushStyle {
        &mut self.pen_slots[self.active_pen_slot]
    }

    pub fn on_hardware_eraser_down(&mut self) {
        if self.hardware_eraser_prev_tool.is_none() {
            self.hardware_eraser_prev_tool = Some(self.active_tool);
            self.active_tool = ActiveTool::Eraser;
        }
    }

    pub fn on_hardware_eraser_up(&mut self) {
        if let Some(prev) = self.hardware_eraser_prev_tool.take() {
            self.active_tool = prev;
        }
    }

    pub fn on_erase_gesture_complete(&mut self) {
        if self.hardware_eraser_prev_tool.is_some() {
            self.on_hardware_eraser_up();
            return;
        }
        if self.back_to_pen_after_erase && matches!(self.active_tool, ActiveTool::Eraser) {
            self.active_tool = ActiveTool::Pen(self.active_pen_slot);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn hardware_eraser_and_back_to_pen_restore_previous_slot() {
        let mut ts = ToolState::default();
        ts.select_pen_slot(2);
        ts.on_hardware_eraser_down();
        assert_eq!(ts.active_tool, ActiveTool::Eraser);
        ts.on_erase_gesture_complete();
        assert_eq!(ts.active_tool, ActiveTool::Pen(2));

        ts.back_to_pen_after_erase = true;
        ts.select_eraser();
        assert_eq!(ts.active_tool, ActiveTool::Eraser);
        ts.on_erase_gesture_complete();
        assert_eq!(ts.active_tool, ActiveTool::Pen(2));
    }
}

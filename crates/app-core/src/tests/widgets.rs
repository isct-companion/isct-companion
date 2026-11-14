use super::*;
use chrono::Duration;

// The launcher is outside Core; capture the kinds it would be asked to redraw.
#[derive(Default)]
struct Launcher(Mutex<Vec<Vec<WidgetKind>>>);

impl WidgetPort for Launcher {
    fn current(&self) -> Result<Option<String>, WidgetError> {
        Ok(None)
    }

    fn replace(&self, _: &str) -> Result<(), WidgetError> {
        Ok(())
    }

    fn clear(&self) -> Result<(), WidgetError> {
        Ok(())
    }

    fn reload(&self, kinds: &[WidgetKind]) -> Result<(), WidgetError> {
        self.0.lock().unwrap().push(kinds.to_vec());
        Ok(())
    }
}

#[test]
fn android_can_cross_the_same_widget_boundary_after_a_clock_change() {
    let before = DateTime::UNIX_EPOCH + Duration::days(1);
    let start = before + Duration::minutes(1);
    let due = before + Duration::minutes(2);
    let mut state = State {
        display: Some(Display {
            boundaries: [vec![start], vec![due]],
            selected_at: before,
        }),
        ..State::default()
    };

    let launcher = Launcher::default();
    for time in [start, before, start] {
        state.advance_display(time);
        state.reload(&launcher).unwrap();
    }
    assert_eq!(
        launcher.0.into_inner().unwrap(),
        [
            vec![WidgetKind::Courses],
            WidgetKind::ALL.to_vec(),
            vec![WidgetKind::Courses]
        ],
    );
}

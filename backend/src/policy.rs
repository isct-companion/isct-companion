use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NativeTarget {
    Desktop,
    Mobile,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCapabilities {
    pub target: NativeTarget,
    pub academic_writes: bool,
    pub foreground_school_sync: bool,
    pub site_script_editing: bool,
    pub site_script_execution: bool,
    pub raw_academic_json: bool,
    pub unattended_schedules: bool,
    pub advanced_merge: bool,
}

impl NativeCapabilities {
    pub const fn for_target(target: NativeTarget) -> Self {
        match target {
            NativeTarget::Desktop => Self {
                target,
                academic_writes: true,
                foreground_school_sync: true,
                site_script_editing: true,
                site_script_execution: true,
                raw_academic_json: true,
                unattended_schedules: true,
                advanced_merge: true,
            },
            NativeTarget::Mobile => Self {
                target,
                academic_writes: true,
                foreground_school_sync: true,
                site_script_editing: false,
                site_script_execution: false,
                raw_academic_json: false,
                unattended_schedules: false,
                advanced_merge: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_keeps_foreground_writes_but_denies_advanced_execution() {
        let mobile = NativeCapabilities::for_target(NativeTarget::Mobile);
        assert!(mobile.academic_writes);
        assert!(mobile.foreground_school_sync);
        assert!(!mobile.site_script_execution);
        assert!(!mobile.raw_academic_json);
        assert!(!mobile.unattended_schedules);
        assert!(!mobile.advanced_merge);
    }
}

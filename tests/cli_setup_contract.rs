use std::collections::HashSet;

use thinclaw_app::{
    ALL_SETUP_WIZARD_PHASE_IDS, ALL_SETUP_WIZARD_STEP_IDS, SetupMode, SetupWizardPlanInput,
    setup_wizard_plan,
};

#[test]
fn legacy_steps_have_exact_target_sections() {
    assert_eq!(ALL_SETUP_WIZARD_STEP_IDS.len(), 27);
    assert_eq!(ALL_SETUP_WIZARD_PHASE_IDS.len(), 10);
    let unique_steps = ALL_SETUP_WIZARD_STEP_IDS
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    assert_eq!(unique_steps.len(), 27);
    let unique_phases = ALL_SETUP_WIZARD_PHASE_IDS
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    assert_eq!(unique_phases.len(), 10);
    for step in ALL_SETUP_WIZARD_STEP_IDS {
        assert!(unique_phases.contains(&step.target_phase()));
    }
    assert_eq!(
        ALL_SETUP_WIZARD_STEP_IDS
            .iter()
            .filter(|step| !step.executable())
            .count(),
        1,
        "only the obsolete continuity prose page is non-executable"
    );
}

#[test]
fn quick_and_advanced_navigation_use_only_target_sections() {
    for mode in [SetupMode::Quick, SetupMode::Advanced] {
        let plan = setup_wizard_plan(SetupWizardPlanInput {
            mode,
            ..SetupWizardPlanInput::default()
        });
        assert!(!plan.phases.is_empty());
        assert!(
            plan.phases
                .iter()
                .all(|phase| ALL_SETUP_WIZARD_PHASE_IDS.contains(&phase.id))
        );
        assert!(plan.steps.iter().all(|step| {
            step.id.executable()
                && plan
                    .phase(step.phase_id)
                    .is_some_and(|phase| phase.step_ids.contains(&step.id))
        }));
    }
}

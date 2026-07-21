# Task for reviewer

[Read from: /home/xifan/code/herdr-corral/plan.md, /home/xifan/code/herdr-corral/progress.md]

Review the CURRENT framework layer of the Rust Herdr sidebar plugin at /home/xifan/code/herdr-corral. Framework-only: host/theme/icons/feature/layout/ui/app, no real Explorer yet.

ANGLE: correctness, API hygiene, error handling, Herdr integration, dead/hacky surfaces that will hurt later.
Read all of src/ carefully (especially app.rs, host.rs, theme.rs, ui/activity.rs, herdr_cli.rs, feature.rs). Note: activity bar half-block chips are intentional TUI technique encapsulated in ui/activity.

Return concrete findings with severity + file/line + suggested fix direction. Do not edit. Separate: must-fix-now vs can-wait-until-feature.

## Acceptance Contract
Acceptance level: attested
Completion is not accepted from prose alone. End with a structured acceptance report.

Criteria:
- criterion-1: Return concrete findings with file paths and severity when applicable

Required evidence: review-findings, residual-risks

Finish with a fenced JSON block tagged `acceptance-report` in this shape:
Use empty arrays when no items apply; array fields contain strings unless object entries are shown.
`criteriaSatisfied[].status` must be exactly one of: satisfied, not-satisfied, not-applicable.
`commandsRun[].result` must be exactly one of: passed, failed, not-run.
`manualNotes` and `notes` are optional strings; an empty string means no note and does not satisfy `manual-notes` evidence.
```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "specific proof"
    }
  ],
  "changedFiles": [
    "src/file.ts"
  ],
  "testsAddedOrUpdated": [
    "test/file.test.ts"
  ],
  "commandsRun": [
    {
      "command": "command",
      "result": "passed",
      "summary": "short result"
    }
  ],
  "validationOutput": [
    "validation output or concise summary"
  ],
  "residualRisks": [
    "none"
  ],
  "noStagedFiles": true,
  "diffSummary": "short description of the diff",
  "reviewFindings": [
    "blocker: file.ts:12 - issue found, or no blockers"
  ],
  "manualNotes": "anything else the parent should know"
}
```
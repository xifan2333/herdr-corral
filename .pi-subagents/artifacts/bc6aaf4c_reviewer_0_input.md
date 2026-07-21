# Task for reviewer

[Read from: /home/xifan/code/herdr-corral/plan.md, /home/xifan/code/herdr-corral/progress.md]

Review the CURRENT framework layer of the Rust Herdr sidebar plugin at /home/xifan/code/herdr-corral. Context: project was intentionally pivoted from monorepo/workbench to a single left-docked sidebar like alexarthurs/herdr-sidebar. Framework is considered mostly OK; user wants cleanup/review before features (Explorer/SCM/GitHub/preview) get heavy.

ANGLE: architecture & shape consistency.
Read all of src/, herdr-plugin.toml, scripts/open-corral.sh, README.md, Cargo.toml. Compare mentally to herdr-sidebar shape (sidebar-only pane, in-process feature switch, preview later separate).

Return ONLY concrete findings, severity (blocker/should-fix/nit), file:line evidence, and why it matters for future feature growth. Do not edit files. End with: top 5 cleanups worth doing NOW before Explorer lands.

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
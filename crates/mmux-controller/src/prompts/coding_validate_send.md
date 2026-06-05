Validation Context

Task: {{task_id}}
Slug: {{task_slug}}
Title: {{task_title}}
Status: {{task_status}}
Project: {{project}}

Objective:
{{objective}}

{{scope_section}}
{{gates_section}}
{{dependencies_section}}
{{blockers_section}}
{{extra_context_section}}
Validation Rules:
- Do not edit files unless explicitly instructed.
- Validate the task against each gate and the stated objective.
- Treat command output, file references, observed behavior, and task state as evidence.
- Distinguish passed, failed, inconclusive, and blocked checks.
- If evidence is missing, say exactly what is missing.

Required Report:
- summary
- gate_results
- evidence
- commands_or_checks_run
- blockers
- unresolved_questions
- recommended_status

---

Validation Instruction:
{{instruction}}

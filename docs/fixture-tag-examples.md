# Fixture tag examples

This page provides copy-paste starter fixtures for the canonical tag types.

Canonical tags:

- `smoke`
- `happy-path`
- `error-handling`
- `edge-case`
- `certification`
- `deferred`

## 1) Smoke + happy-path

```yaml
schema_version: 1
skill_or_agent: hello-skill
cases:
  - id: hello-skill/happy-path
    tags: [smoke, happy-path]
    input:
      messages:
        - role: user
          content: "Say hello"
    expected:
      tool_sequence: []
      output:
        - kind: contains
          value: "hello"
```

## 2) Error-handling

```yaml
schema_version: 1
skill_or_agent: hello-skill
cases:
  - id: hello-skill/error-empty-input
    tags: [error-handling]
    input:
      messages:
        - role: user
          content: ""
    expected:
      tool_sequence: []
      output:
        - kind: regex
          value: "(?i)(provide|empty|missing)"
```

## 3) Edge-case

```yaml
schema_version: 1
skill_or_agent: hello-skill
cases:
  - id: hello-skill/edge-long-input
    tags: [edge-case]
    input:
      messages:
        - role: user
          content: "<very long text omitted>"
    expected:
      tool_sequence: []
      output:
        - kind: contains
          value: "summary"
```

## 4) Certification

```yaml
schema_version: 1
skill_or_agent: compliance-skill
cases:
  - id: compliance-skill/cert-baseline
    tags: [certification]
    seed: 123
    input:
      messages:
        - role: user
          content: "Assess this control"
    expected:
      tool_sequence: []
      output:
        - kind: contains
          value: "assessment"
```

## 5) Deferred placeholder

```yaml
schema_version: 1
skill_or_agent: blocked-integration
cases:
  - id: blocked-integration/deferred-placeholder
    tags: [deferred, placeholder]
    input:
      messages:
        - role: user
          content: "Is this fixture deferred?"
    expected:
      tool_sequence: []
      output:
        - kind: regex
          value: "(?i)(deferred|blocked|pending)"
```


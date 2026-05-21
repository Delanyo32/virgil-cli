# Migration issues

17 vertical-slice issues for the Datalog-schema migration. Ready to be pushed to GitHub as separate issues; each file is a standalone issue body.

## Dependency graph

```
#1 (schema swap)
├── #2  Rust baseline
├── #3  TypeScript baseline
├── #4  Python baseline
├── #5  Go baseline
├── #6  Java baseline
├── #7  PHP baseline
├── #8  C baseline
├── #9  C++ baseline
└── #10 C# baseline
       ↓ (all 9 baselines)
       ├── #11 Parameter/local symbol extraction
       └── #12 Symbol metadata (visibility/qname/parent/modifiers)
              ↓
              ├── #13 Class hierarchy + signatures + Level-3 types
              │       └── #14 field_type relation
              └── #15 Per-language *_attrs tables

#11 + #13 → #16 References (Level 3)

#12 + #13 + #15 + #16 → #17 Template rewrite
```

## Push to GitHub

When ready, push in dependency order so blocker references resolve:

```bash
# Start with #1
gh issue create --title "Schema swap to new Datalog model (foundation, empty extraction)" \
    --body-file docs/issues/001-schema-swap-foundation.md --label enhancement
# Repeat for 002–017
```

Or use a small loop:

```bash
for f in docs/issues/0*.md; do
  title=$(head -1 "$f" | sed 's/^# //')
  gh issue create --title "$title" --body-file "$f" --label enhancement
done
```

Note: the `Blocked by` references in each issue body use the sequence numbers from the filenames (`#1`, `#2`, ...). If GitHub assigns different issue numbers (e.g. issues already exist), update the `Blocked by` lines in the issue bodies before pushing.

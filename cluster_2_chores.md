## Cluster 2 — Chores & Home Maintenance

*Combined replacement for §7 (Chore Rotation & Fairness) and §17 (Algorithms/fairness), and answers to §21E (questions 15, 16, 17). Resolves the contradiction between §7's auto-rotation engine and §C's "the system is not a work distributor". §C wins.*

### 1. Posture

The system records chores, displays the schedule, logs completions, and stays out of the way. It does not:

- Auto-assign chores to specific people via any algorithm.
- Compute a fairness score, decay function, or load-balance penalty.
- Suggest reassignment based on perceived imbalance.
- Apply emoji, colour-coding, or escalating tone to overdue items.

It does:

- Show whose name is currently on each chore (a default, freely changeable).
- Let any household member change the current assignee in one tap, no friction, no required reason.
- Keep a completion log: who marked it done, when. Visible per-week.
- Surface the work that needs doing today and this week, in the same gentle stream as everything else.

Fairness is something humans do to each other. The system makes the facts visible. The humans negotiate.

### 2. Entity model

No new entities are needed. Chores are `Task` instances (per §3.5) with:

- A recurrence rule (the cadence).
- An optional list of `eligibleMemberIds` (e.g. "any adult"; "Sam or Alex"; "anyone over 10"). Empty means anyone.
- A `currentAssigneeId` — a default, not an assignment. Anyone can change it; the system never changes it on its own.
- Tags from the pre-seeded set or free-form.

When a chore instance is completed, the `CompletionLog` record is written:

```
CompletionLog {
  id, taskId, instanceDate,
  completedBy, completedAt,
  notes?, skipped?: bool
}
```

This is the only new structure introduced by Cluster 2. It is the substrate for the *visible record* the household uses to talk about who's been doing what — without the system itself drawing conclusions.

### 3. Reassignment and skipping

- **Reassign**: any household member can change `currentAssigneeId` on any chore instance, at any time. No reason required, no notification fired, no log entry treating it as a special event. The change is just a change.
- **Skip**: marking a chore skipped writes a `CompletionLog` entry with `skipped=true` and an optional one-line note. The chore does not roll forward "with lower priority" or accumulate guilt-debt as the original §7 suggested. It just didn't happen that week.
- **Done by someone other than the assignee**: completely fine. The log records who actually did it. The default assignee for the next instance is unaffected unless someone changes it.

### 4. Pre-seeded tags

The system ships with a small set of suggested tags, surfaced in the tag picker. None are required:

- `chore` — recurring household upkeep.
- `errand` — out-of-house, one-off or low-cadence.
- `maintenance` — long-cadence items, often connected to a Project.
- `outdoor` — affected by weather; UI may show a weather hint.
- `kid` — visible in Kid Mode views.
- `garden`, `seasonal` — convenience tags for the gardening and seasonal-cluster cases.

Users add their own freely. The system uses tags only for default behaviour (e.g. holiday-skip for `chore`, weather hint for `outdoor`, kid-mode visibility for `kid`) and never to enforce a taxonomy.

### 5. Seasonality (NL context)

Specific recurring items for a Dutch household, mapped to model elements. This is illustrative, not exhaustive — every household will tune this.

| Item                          | Cadence              | Modelled as                                       |
| ----------------------------- | -------------------- | ------------------------------------------------- |
| Chimney sweep                 | yearly, October      | Recurring Task, tag `maintenance`                 |
| Gutters cleaned               | yearly, November     | Recurring Task, tag `maintenance` `outdoor`       |
| Boiler service                | yearly (contract)    | Recurring Task: "verify service done & file invoice" |
| Bike spring tune-up           | yearly, March        | Recurring Task                                    |
| Bike autumn lights/tyres      | yearly, October      | Recurring Task                                    |
| Heating-on / heating-off      | twice yearly         | Two recurring Tasks                               |
| Storm windows / draft check   | yearly, October      | Recurring Task, tag `outdoor`                     |
| House repaint                 | every 7 years        | Project, with child tasks (estimates, quotes, scheduling, prep, painting) |
| Garden — yearly plan          | seasonal             | Project ("Garden 2026") + soft membership of recurring tasks |
| Garden — recurring care       | bi-weekly in season  | Recurring Tasks, optional `projectId` to current year's garden project |
| Sinterklaas preparation       | yearly, November     | Project (gifts, schedule, food)                   |
| Vacation preparation          | per trip             | Project                                           |

#### Projects and recurring tasks: soft membership

A Project (e.g. "Garden 2026") may have recurring tasks soft-grouped under it via a `projectId` field on the Task. The Project does not *own* the tasks: archiving or deleting the Project does not delete the recurring tasks. This lets a household either work with a parent project (yearly garden plan with all child tasks visible together) or skip the parent entirely (just recurring tagged tasks that show up in *Today*). The data model supports both with the same shape; only the user's choice to set `projectId` differs.

### 6. Supplies coupling (deferred to Cluster 3)

Chores do not directly add items to grocery lists. The coupling is *pantry-driven*: certain items have low-threshold pantry records, and either explicit checks (a checklist step on a chore that asks about a supply level) or manual updates lower the pantry value. When a value drops below threshold, the item lands on the grocery list automatically.

Cluster 2 commits only to this hook on the chore side: **Tasks can carry checklist items that prompt about supply levels**. The receiving side — `PantryItem`, thresholds, list materialisation — is defined in Cluster 3.

### 7. What changed from §7 and §17

- Removed: argmin auto-assignment, rolling 28-day effort score, fairness penalty term, decay λ, "give a break" suggestions, "fairness meter" UI.
- Removed: cascading roll-forward of missed chores with lower priority.
- Kept (and clarified): rotation as a *display* concept — the system shows whose name is on a chore, but the value behind the name is just a default that humans change as they wish.
- Replaced: the fairness *algorithm* with a fairness *fact* — the completion log, visible per-week, with no derived score.
- Added: `CompletionLog` entity. `eligibleMemberIds`, `currentAssigneeId`, soft `projectId` on Task.
- Added: NL seasonal reference list, modelled entirely within existing entities.
- Deferred: supplies coupling to Cluster 3 (pantry).

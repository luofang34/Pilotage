# ADR-0052: Mission objectives, approved plans and recorded runs

- Status: Proposed
- Date: 2026-10-22

## Context

A mission reaches the session host in one way: the operator sets
`PILOTAGE_MISSION_*` variables before the host starts
([ADR-0025](0025-client-optional-operation-automation-principals.md)). The
mission principal builds one document from them
([ADR-0041](0041-one-mission-sequencing-core.md)), flies it, and ends. The
operator cannot issue a mission from a client, change it in flight, or
issue the next one without a host restart. A client shows the mission only
through the authority events and the guidance group. An area survey is one
`FollowPlan` phase, and the mission core cannot see its lines. A finished
route is the only record of the flight; nothing records which pictures the
survey acquired. The agent
([ADR-0042](0042-agent-client-module-and-model-port.md)) has a directive
vocabulary and a lowering onto flight actions that its executor cannot
run, because the document does not change in flight. The physical-vehicle
lane ([ADR-0051](0051-a-physical-vehicle-flies-on-its-own-estimate.md))
made the operator's decisions part of a mission's life and expressed them
through leases only. A planning library with navigation search, route
drafts, vehicle profiles and coordinated drafts exists as a draft and
grants no authority.

The targets, in order, are one Mini 3 survey, an approved multi-vehicle
survey, and a coordinator that works without a connection to the operator
and adapts within limits the operator delegated. The design must keep each
concern in one record and one owner, so that the mission system grows for
years without a rewrite: what the operator wants, how a plan satisfies it,
who permitted what, what the vehicle did, and who may change it.

## Decision

### Five records, not one

A mission is five distinct records. Each record that can leave a session
follows [ADR-0049](0049-collaboration-ready-records-and-authority.md): a
global identifier, an issuer-qualified origin, an append-only revision with
the digest of its predecessor, validity, and releasability.

- **Objective.** What the operator wants: for a survey, the area, the
  required coverage and image quality, the camera assumptions, the
  exclusions, the completion evidence, and the time and energy limits. A
  route is one way to satisfy an objective. A change of plan keeps the
  objective and the evidence of work already done. A finished route is not
  proof that the objective is met.
- **Proposal.** A compiled plan with its estimates and the assumptions it
  rests on: navigation data revision, vehicle capability snapshot, energy
  model, execution strategy. A proposal is grounded and reviewable, and
  has a digest. It grants nothing. A proposal travels as an immutable
  record: the lane that compiles it encodes it one time, and the record
  carries these bytes, their format, and the digest of these bytes
  (ADR-0014). Each receiver checks the digest against the bytes that it
  received. It never hashes a new encoding of the decoded proposal. A team
  proposal carries the record of each member, with the bytes of the
  member unchanged. Two proposals with the same meaning can have
  different bytes, and then they have different digests; no canonical
  encoding is defined. A receiver refuses a record format that it does not
  know. A proposal lists the execution features that it needs, and an
  approver that does not know one of them does not approve the proposal.
- **Approval.** The operator's or a delegate's permission, bound to one
  proposal digest, the format of its record, and its assumptions. A lane
  refuses an approval that names no record format. It carries an adaptation envelope:
  the permitted vehicles, the area and height bounds, the minimum energy
  reserve, the permitted payload uses, the expiry, and the kinds of
  revision it allows. It states the permissions separately: to execute, to
  revise, to allocate work, and to change the objective. It also names
  the recovery for its expiry: hold in place, or end after the line in
  progress and return to the launch point. When the approval expires
  during a run, the lane applies that recovery one time. Then the run does
  not resume, and an admitted revision does not apply, until a renewal of
  the plan replaces the expired approval.
  An approval names its target by its digest and its run. With no run, it
  approves the plan before its start. With the run in progress, it renews
  the plan of that run, and a lane refuses it for another run. This
  prevents an approval of one run from renewing another run of the same
  plan. An approval of a drafted successor names the run in progress or no
  run. A vehicle lane and a team find the target with the same rule.
  At each entry point, for a plan, a renewal or a successor, they also
  make the same checks before they change anything or pass anything on:
  the approval names the record format, permits execution and has not
  expired. Limits of the domain, such as height and area, are checked
  after these checks, by the domain. A team passes its approval down to a
  member only when the approval is valid at that time.
  A held run reports each reason why it holds: the operator, or the
  expiry of its approval. A renewal removes only the expiry reason, and an
  operator's resume removes only the operator reason. The lane refuses a
  resume while the expiry reason stays, and the run resumes when no reason
  stays. A fault, a motion lease held elsewhere and an unknown member state
  are not hold reasons, and a renewal does not clear them. A team checks
  its envelope and its run, then sends each member that can still fly a
  renewal of the plan and the run of that member. A member that finished
  gets no renewal. A member whose link is lost, or that a person stated
  down, gets none, and the receipt of the team names it. The team answers
  after its members answered. A team reports `Held` only when each member
  that can still fly reports that it holds, and it reports the reasons of
  all those members.
  Each lane status lists the approvals that the lane needs: its plan
  before the start, a drafted successor, and the renewal of an expired
  approval. A team names itself on each approval that it passes down to a
  member, on each objective that it makes for a member, and on each share
  of its revision that it sends to a member. The host takes these names
  only from a session with the approver secret. The team owes the approval
  of the plan that it made for a member, of its own share of a revision
  when it granted the approval in force, and the renewal of what it
  granted; the need names the team. A lane lists a need only while it can
  take that approval. A client raises a need for a person only when no
  lane owes it. It does not find this from the name of a session.
  A client approves only a proposal that it holds in full. The lane's status names a
  proposal only by its digest and a summary, so a client that did not
  make the proposal fetches it from the lane before it approves it.
- **Plan.** The executable `MissionDocument`
  ([ADR-0041](0041-one-mission-sequencing-core.md)): immutable,
  digest-identified, bound to a verified execution strategy and its
  link-loss behaviour. A plan references the objective and the approval it
  executes under. A successor plan names the plan digest it supersedes.
- **Run.** What the vehicle did: the run identifier, the checkpoints, the
  action identifiers, the receipts, the observations, and the outcome of
  each task, including partial completion, refusal, and an unknown
  outcome. The run lives outside the immutable plan. A digest chain alone
  does not say what the aircraft executed; the run does.

### One lane per vehicle, at the vehicle's control endpoint

Each vehicle has one mission lane and one authority arbiter at its control
endpoint: the host that holds the control path to the vehicle
([ADR-0028](0028-multi-vehicle-and-swarm-coordinator-hosts.md)). For the
Mini 3 that endpoint is the host attached to its remote controller, not
software inside the aircraft. A flight controller or a companion computer
may host the deterministic executor when its platform exposes a verified
command interface and its resources permit; it need not host a model. The
lane has a lifecycle: `Idle`, `Admitted`, `Running`, `HoldRequested`,
`Held`, `Uncertain`, `Complete`, `Aborted`. An accepted hold makes the lane
`HoldRequested`; the lane becomes `Held` only when the vehicle reports that
it holds in place. An abort keeps each task that did not finish as
unfinished: the execution of the task in progress becomes `Cancelled`, and
a pending task stays pending. An operation task keeps its execution until
the host reports how the operation ended, because the operation can still
have its effect: the host cancels it as its registered cancel semantics
say, and then reports it cancelled, completed or failed. An abort of a team also ends the work that
the team was still setting in motion. A re-flight or moved lines that have
not started are cancelled, and a receipt that arrives later takes no next
step. When the start was sent, the member may fly, so the work keeps its
member until the member reports. Committed work that its receiver had not
started is unassigned when its giving member released it. Work that moved
on a person's statement, without a release, is unreconciled instead,
because the giving member may still fly it. A release fact that arrives after the abort is
recorded as unassigned work, and it gives no work to the receiver.

Each kind of objective plans in its own module, and what follows the plan
is the same for each kind: its approval, its run, its evidence and its
judgment. A task that is one operation of a host executor names the
executor, the operation, its version and its parameters (ADR-0041). A
parameter is a value or a result of an earlier task of the plan. The lane
refuses a plan when it is proposed if its host does not register each of
its operations in that version, with parameters of the declared types. The
status gives the results of each task and the question that an operation
asks a person. A person answers with the operation's identity and the
digest of the subject that the person saw, so the lane refuses an answer
to a subject that changed. The host takes an answer only from a session
with the approver secret, and it writes the session that sent it. A relay,
such as a team, names the person that it passes the answer on for, as its
own host wrote it, and the record names both. An operation task's judgment
is met when its execution completed and the result that the plan names has
its value. A report objective reads a value, asks a person to confirm it,
and writes a report of it: a person who declines ends the run, and the
report is not written. A check objective reads a value, a rule holds the
value to a limit, and the host writes a report of the verdict; no person
takes part. A rule is a pure operation that every host carries out. A kind
of objective is its planning module and the executors that it names. The
lane registers each new kind's planner by name, with its version and its
typed parameters, and a named objective carries those parameters, so a new
kind needs no wire message of its own. Every kind but the survey, such as
the report and the check, is a named objective and plans through the
registry; the survey, whose area a team divides, plans in its own module. Lifecycle, authorization, coordination, recovery
and display treat every kind that is not a survey the same, and
`scripts/check-objective-kinds.sh` refuses code outside planning that
names a kind. A client proposes an objective of any kind with one command
that carries the encoded objective. The host writes the report of one
operation once, so a repeat of that operation finds it. A plan of
operations has no way home: it takes no revision, and an expired approval
holds it before its next operation. The host keeps a record of the lane's
run in a journal: the approved plan, the approval, the run, and the
execution and results of each task. The record does not keep evidence or
display. The host starts no run before the journal holds it, and a run of
operations begins an operation only when the journal holds the end of each
operation before it. Thus, while the journal cannot be written, no new
work begins; a hold, an abort and an answer still pass. After a restart,
the lane starts from the record and starts no work. The descriptor of the
executor of each operation that did not end sets what occurs. If the host
can tell whether the operation occurred, the host asks the executor. If a
repeat with the same identity changes nothing, the operation runs again.
Any other operation becomes `Uncertain`, and a person decides; the absence
of a receipt does not show that the operation did not occur. When no task
is uncertain, the run holds with the reason `Restarted` until a person
resumes it. The resume starts the remaining tasks under the same run
identity. A flight does not fly again after a restart, because the lane
cannot know where the vehicle is: the lane stays `Uncertain` until a
person aborts the run. A run that ended keeps its outcome, so a restart
does not replay cancelled work. A team gives an objective with no area to
divide, such as a report, to one member whole, as an objective that the
team made. The team follows that member's run as any other. Its status
shows the question that the member's run asks. The team gives an answer to
the member that asks, as an operation, and the operator gets one receipt
when that member answers. The environment variables remain a bootstrap
source that issues the first plan into an `Idle` lane; an issued plan
supersedes them.

### Tasks are the nodes

A node is a bounded task with its preconditions, its outcomes, its
deadline, its resource claims, and its cancellation behaviour. One survey
line with its acquisition is one task. A task has an opaque identity that
is unique in its proposal, and a name for display. No behaviour follows the
name or the position of a task in a list. The mission core runs each task
as a phase with the identity of the task, and the lane follows the task
events of the core. A run that a revision starts names the run that it
continues, so a team follows the work of a member through its revisions
without the use of the names of runs or tasks. A route leg is not a task: Navigate
keeps the sequencing of legs inside the task, and flight and camera
acquisition run together under it.

A task status has three facts, and each fact has one source:

| Fact | Values | Source |
|---|---|---|
| Execution | `Pending`, `Active`, `Completed`, `Cancelled`, `Failed`, `Skipped` | Run events: a task finishes, an abort, an unplanned end of the run, a revision |
| Evidence collection | `Open`, `Settled`, `Closed` | The pictures of the task, the answers to them and the evidence window |
| Judgment | `Undetermined`, `Met`, `NotMet` | The rule of the task kind, from the execution and the evidence |

Evidence never changes the execution. A verdict that arrives after the
execution ended can change the evidence and the judgment. It cannot make a
cancelled or failed execution complete. A survey line is `Met` only when
its execution is `Completed` and its evidence meets the minimum, because a
line cut short does not cover its swath. On a lane with a judgment service,
the minimum is a number of pictures judged usable, and a line that nobody
judged is not usable. The way home is `Met` when its execution is
`Completed`. A display state (`Done`, `Incomplete` and the others) comes
from the three facts. A lane, a team, a client or an agent makes its
decisions from the three facts, never from the display state. A step that
waits for a task names the fact that it waits for: the next survey line
waits for the end of the execution of the line before it, and a report
waits for the judgment.

Each picture that a lane sends for judgment carries its run, its task, the
attempt and the picture number. The lane gives a verdict to the run that
sent the picture. This is also true after a successor replaced that run. The
lane refuses a verdict for a run that it does not know, for a picture that
it did not send, or a second verdict for one picture. The evidence of a
task is `Open` while the task takes pictures and while a sent picture has
no answer.

The evidence window of a task starts when its execution ends. From then on,
the task takes no more pictures, so the set of pictures that can get an
answer is closed. The window has a length from the lane configuration. This
length is a deployment default, not a validated value. The window uses the
monotonic clock of the host. It does not use the mission clock (ADR-0041),
because the mission clock stops while the run holds, and a held run must
still close its windows. When the window closes with answers outstanding,
the evidence becomes `Closed`, and the judgment uses the answers that
arrived. The lane counts an answer that arrives after that as late. A late
answer changes no other fact. At most four replaced runs wait for answers
at one time; when one more run is replaced, the windows of the oldest close
at once. The evidence is not persisted. A lane that a host restores after
a restart refuses verdicts for the pictures of the runs before the
restart, and the host never compares the monotonic times of two
processes. A display may show progress inside a task from
the remaining waypoints, and that view is not a second execution truth. The
first delivery uses a sequence of tasks per vehicle. A team task graph and
concurrent tasks within a bound come later; a linear list is not promised
to cover every mission.

### Planned change at a checkpoint, urgent action by preemption

A planned change is a successor plan. It is bound to the active plan
digest, the run identifier, and the checkpoint it was planned against; the
lane refuses it when any of these changed. The lane admits a successor
while a task runs and switches at the next checkpoint: the completion of
the running task, or at once while the lane is `Held`. The lane never
splices a running plan. A switch does not end a hold: the run of the
successor starts held, and an operator's hold or a restart's hold stays
until a person resumes. Only the hold that an expiry applied ends when the
successor's own approval admits it. The return home that an expiry names
resumes under the expired approval.

The run in progress and an admitted successor each hold their own approval,
bound to its own digest, run and expiry. A successor is admitted on one of
three bases: an approval of its own digest, the approval in force inside its
envelope, or the recovery that an expired approval names. A renewal of the
run changes only the approval of the run; an approval of a successor changes
only that of the successor. Each approval expires on its own, so the run
applies its own expiry policy whatever the approval of the successor. At the
checkpoint, the lane checks the approval of the successor again against the
successor. When that approval no longer admits it, the successor waits for a
new approval. At the switch, the approval of the successor becomes the
approval of the run.

An urgent action does not wait for a checkpoint. `Hold`, `Return`, `Land`
and `Abort` are preemptions: the lane requests the stop, observes the
vehicle's transition, and applies the change only when the transition is
known. When it is not known before the deadline, the lane enters
`Uncertain` and says so. The four actions have different outcomes and
prerequisites: `Return` needs a valid home position, a feasible route,
sufficient reserve, and a suitable endpoint. The recovery policy the lane
will apply is visible in the status at all times. `Skip` validates the
successor's prerequisites and reports the coverage it leaves unmet. A
reconnect never replays an action with an unknown outcome.

### Approval is explicit, or covered by an explicit delegation

Nothing executes without an approval that binds its proposal. A successor
inside the approval's adaptation envelope is admitted locally, by the lane
or by a coordinator, without another operator action. A successor outside
the envelope needs a new approval. When no approval can be obtained, the
lane applies the recovery behaviour the approval already authorized; it
never changes the objective to declare success.

A successor is inside the envelope only when the envelope permits each
class of change that the successor makes. The lane or the coordinator finds
the classes when it compares the old content with the new content. It never
uses the kind of request as the class. The classes are:

| Class | What it changes | Permission |
|---|---|---|
| Plan | How the work is done: the tasks, their order, the speed, the height when no constraint fixes it | `permit_revise` |
| Assignment | Who holds the work | `permit_allocate` |
| Objective | What the work must achieve | `permit_change_objective` |

One successor can make changes of more than one class. Then each of the
permissions is necessary, and a refusal names each missing permission. A
vehicle lane and a team use the same check. The task kind gives the
meaning of its fields. For a survey, the area, the return to the launch
point and the evidence that each line needs are objective fields. The
height is a plan field, but it is an objective field when the objective
holds the height (`hold_height`). The height bound of the envelope applies
to all plans. The title, the identity and the revision number of an
objective are not content: an objective with the same identity and
different content is an objective change. A survey names its area by
identity and revision when the area is a named object, such as an area
drawn on a map. Another identity of the area is an objective change, also
with the same vertices. The vertices tell whether the area changed. The
revision of the area object and the text that a ref shows are not content.
An area found from labels has no identity of its own, so its vertices are
all it is. The strip that a team gives to a member is part of the area, so
it names no area identity. An explicit approval of the digest of a draft
is the permission of the approver for that draft, so this check applies
only to admission inside the envelope. Task-by-task consent is an optional
workflow inside this rule, not a separate level.

A team that passes its approval down divides a consumable budget; it
does not copy it. A member team gets an even part of the re-flight budget,
and the first member teams get one more line each until the remainder is
gone. When the team has vehicle members too, it keeps one part for their
lines. A vehicle member decides no re-flight, so it gets no part. An extra
flight that the team pays for, a re-flight or moved lines, goes down with
no budget, so no part is spent two times. The parts never add up to more
than the approval of the team allows.

The end of the runs of a team and the settlement of its results are
different facts. When every member's run ended and the evidence of a line
is still open, the team waits for its results. It shows the state
`AWAITING_RESULTS`. A judgment that comes inside the evidence window gets
the same decision as a judgment that came before the runs ended: the
team flies the line again when the judgment is not met, the budget
allows, and the approval is valid at that time. When the approval is not
valid, the team records the judgment and flies nothing. After an abort,
the team records results and starts no work. The team ends only when no
evidence of its lines is open. Evidence of a member that a person stated
down does not keep the team open.

Bounds copied into a plan grant nothing. The lane verifies a plan against
the issuer's grant at admission and again during execution. A delegation
that must hold without a connection has an expiry and a preauthorized
behaviour for the time when renewal or revocation cannot arrive. A
reconnect reconciles state; it does not make an unseen revocation timely,
and it does not restore a revoked grant. The mission principal holds
bounded authority that stays valid without a client heartbeat, distinct
from an interactive control lease; the vehicle's own authority arbiter
and a manual override fence every action regardless. A native
remote-controller override holds the mission and revokes software control
until the operator authorizes it again.

Work, control and recovery have separate lifetimes. An approval permits
the work; a lease gives temporary control; a recovery is what the approval
permits when no person controls the vehicle. When a principal takes the
lease from a run, the work stops, and only a hand-back resumes it. While a
person holds the lease, nothing moves the vehicle against that person. When
the taker's lease ends without a hand-back, by a release or a closed
connection, the run applies the approval's `on_control_lost` recovery, if
the approval is still valid: `HOLD` waits for a hand-back, and
`RETURN_HOME` takes the free lease only to fly home inside the approval's
limits. A revocation of the run's own lease, with no taker, is a stop and
starts no recovery. After the approval expires, only its `on_expiry`
recovery applies. A recovery run flies a home-only plan and never runs the
work again. Admission refuses a recovery that the plan cannot fly: a plan
of operations has no way home. When the link between the host and the
vehicle fails, the host can do nothing, and the vehicle-local fallback
that its adapter advertises applies. For the DJI adapter, that fallback
is `Neutralize`, which stops software motion and guarantees neither a
hover nor a return home.

### Proposal and admission are separate, and planning is shared

`Propose(request)` is separate from `Issue` and `Start`. The host compiles
a request into a grounded proposal with its estimates and returns it for
approval. The planning library is shared code: a client uses it for an
offline draft and a preview, and the host uses the same code to validate
the exact approved proposal against the current state before it admits
anything. "No client plans" prohibits independent planning logic and
independent authority in a client; it does not prohibit reuse of the
library. The host serializes admission and owns execution deadlines and
action outcomes.

Admission checks the execution strategy. A host-guided route needs timely
velocity control and navigation feedback; a vehicle that executes an
uploaded native plan has a different contract: plan admission, start, hold
and cancel, progress, and local recovery, with the update semantics the
device verifies. A plan is bound to one verified strategy and its
link-loss behaviour, and a strategy change needs renewed validation.
Admission needs the vehicle's resource and capability references: usable
energy and reserve, payload and capture capabilities, localization
integrity, dynamics, link behaviour, and verified recovery actions. A
missing input yields an unresolved estimate or a refusal, never a default.
Units, altitude datum, data revision and uncertainty survive every
handoff.

### Intents, receipts and one status

The decisions are intents
([ADR-0047](0047-agents-read-one-context-port-and-act-through-intents.md)):
`Propose`, `Issue`, `Start`, `Hold`, `Resume`, `Skip`, `Revise`, `Return`,
`Land`, `Abort`. One vocabulary serves a client's mission panel, a gamepad
profile, a coordinator, and an agent. The authority engine checks each
intent under a mission scope. Every intent gets a receipt: admitted and
the checkpoint it applies at, held in `Uncertain`, or refused with the
reason. The host publishes one mission status on the reliable stream when
it changes and at a slow steady rate: the objective and approval
references, the active plan digest and its chain, the run identifier and
checkpoint, the lane lifecycle, the task list with each task's state and
evidence, the admitted successor and the checkpoint it waits for, the
recovery policy in force, the estimates, and the last receipt. The same
status is the mission entry of the agent context port. A client keeps its
mission object across a reconnect by the digests and resynchronizes from
the status.

### A coordinator is a deployable role

A coordinator works through ordinary vehicle hosts
([ADR-0028](0028-multi-vehicle-and-swarm-coordinator-hosts.md)) and may
run on the operator's computer, on another computer attached to a remote
controller, or on a leader's companion computer. A team proposal is a
task graph with per-vehicle assignments; each assignment names its
vehicle, its capability snapshot, its region and time reservation, its
plan, its authority epoch, and its expiry. The coordinator admits the
required participants before coordinated execution starts, and the
proposal states what happens when only some participants admit or start;
a multi-vehicle start is not atomic.

A coordinator request that waits for its members is an operation. The
operation records the operator's request, the member requests that it
sent, and the result of each. Only one operation of a kind is open at a
time. The coordinator refuses a second operation of that kind as busy,
because it does not put concurrent operations in order. Each operation
gets one receipt: when its members answer, when an abort or a new plan
ends it, or when an answer is lost. A member request that did not go out
is refused, and the receipt says that it was not sent. An answer that is
lost, because the link is lost or no answer comes in time, makes the
receipt uncertain. It does not show that the member did not execute the
request. Thus an answer that comes after its operation ended still
changes the team, but it does not give the operator a second receipt.
The team takes an abort also while it waits for the members' proposals or
approvals, so that a lost answer does not keep the team in that state.

Three losses are distinct: loss of the internet, loss of contact with the
operator, and loss of the control link between a coordinator and a
vehicle. Approved authority and local resources let a coordinator continue
through the first two. A vehicle whose control path needs a host and a
remote controller cannot continue a mission after that path fails, and an
airborne leader needs a real transport to each follower.

In a partition there is one accepted writer per vehicle and per
assignment. A new coordinator fences the old one at the vehicle's control
endpoint; a new name alone is insufficient. The work and the space
reserved for an aircraft that cannot be contacted are not reassigned as if
it had stopped; reassignment needs a release acknowledgement, a valid
expiry with fencing, or work preallocated as disjoint. A team lane keeps
the work of a member with a lost link, and its line in progress, reserved
to that member until the member reports again or an approver states, as a
separate act, that the member is down.

A handover between two members has four separate facts: the link (can the
team contact the member), the execution (does the member still fly), the
assignment (which member holds the work, in which generation) and the space
(which area the work needs). A move of work between two members that both
report has one commit point. The team asks the giving member for a release
of one assignment in one generation, in the revision that it sends to that
member. When the lane of that member applies the revision, its status
reports the release fact: the assignment, the generation, the digest of the
revision, the replaced run and the tasks that the replaced run did not
begin. The team commits the work to the receiving member, in the next
generation, only when the fact names the open handover, the revision that
the team approved, and the run that it asked. Before that, the receiving
member gets nothing, and the work and its space stay with the giving
member. The team does not use a changed run or a shorter list of tasks as
a release. A moved task that the giving member began before the revision
applied is not in the release, and it stays with that member. Without a
release fact, the handover stays open and the status shows it. A team
that is a member of another team does not make a release fact from
its members' facts. It refuses a revision that asks for a release before
it changes anything, and its parent withdraws the handover. A node does
what an operation asks, or it refuses the operation. Nothing is
assumed at a time-out. When the giving member refuses its revision, the
handover is withdrawn and that member keeps the work. When the receiving
member refuses the released work, or a person states it down before it
starts the work, the work is unassigned. It does not go back to the giving
member. When the work moved on a person's statement and not on a release,
it is unreconciled, as the giving member may still fly it. A lane refuses an approval or a revision for a plan that a
revision replaced, so a late message cannot bring a released plan back.
Fencing at a control endpoint (ADR-0010) makes an old generation invalid
at that endpoint only: a lease on the receiving vehicle does not stop the
giving vehicle.

A person's statement that a member is down is an override with a source.
The team records who stated it, and the handover that it causes names it
as its basis. The statement does not prove that the member stopped, that
its control is revoked, or that its space is clear. The coordinator host writes the
handovers to durable storage before it sends work to a member. An intent
that gives work is prepared, then persisted, then sent, and never sent
before its record. When a write fails, the prepared intents wait in a
bounded backlog, the coordinator refuses new work and says so in its
status, and it tries the write again. When the write succeeds, the waiting
intents go out one time. A hold and an abort never wait for the journal.
A hold for a member with waiting work goes out at once, and it also waits
again after that work. When the write succeeds, the member gets the work
and then the hold, in the same order as when the journal is written. Thus
the disk does not change what a hold does. An abort for a member removes
the waiting work of that member. When the work of one answer does not fit
in the backlog, all of that work is removed, so that no part of it goes
out without the rest. The team lane receives each removed intent as
refused by its member, because the member never received it. Thus the lane
does not show the work as possibly started. A coordinator that restarts
reads them back. It shows each handover that was open or committed as
unreconciled, because it cannot know whether a member flies that work,
and it moves nothing. While a handover of its members is unreconciled, the
team refuses new work. A person checks both members and then resolves the
handover with an explicit intent; a new proposal does not resolve it.
Handover records outlive the plan that they came from: a new proposal, a
failed proposal and the refusal of a member keep every record that is not
settled. A record is settled when it is withdrawn, when the receiver flew
the moved lines, or when a person resolved it. Only a settled record is
dropped. When a team starts new work after its run ended, a handover that
is still open or committed has no run to follow, so it becomes unreconciled
too. A flight of moved lines carries the identity of its handover, and the
team matches the flight to that handover only. An assignment identity includes the run of the team, so a
restarted coordinator never uses an identity or a generation again. It
has no run of the team, and it can give work only through a new team
proposal that each member must accept. A member lane refuses a proposal
while it flies.

The team reports each member's
link as a typed state (`Reporting`, `Lost`, `Down` or `Released`), and a
vehicle's lane reports what its run does now: flying, waiting for a
launch, waiting for a lease that another principal holds, holding, or
stopped. Clients act on these states; the detail text is for display
only. Loss of progress is preferred to conflicting control. Deadlines bind to a host's boot and
clock identity, and schedules across vehicles record their synchronization
uncertainty. Each deadline names its clock and whether a hold stops it:

- An approval expires at a civil time, in Unix milliseconds. A hold does
  not stop it, and an expired approval holds the run.
- The evidence window of a task starts when its execution ends and runs
  on the host's monotonic clock. A hold does not stop it.
- A team waits 30 s for its members' answers on the host's monotonic
  clock. A step of the civil clock does not change the wait, and a hold
  does not stop it.
- The receipt timeout of a mission action and the wall deadline of a run,
  24 hours from its start, run on the host's monotonic clock. A hold does
  not stop them. A receipt that came during the hold is taken before the
  receipt timeout is checked, so only a directive whose receipt never
  came times out at the first step after the resume. The wall deadline
  and the phase deadlines are checked before any receipt.

A host restart reconciles state; it does not replay commands.
When an endpoint dies or loses its aircraft link, its adapter reports the
device's verified loss behaviour or refuses the autonomy requirement.

Allocation has one interface and replaceable solvers. The first solver is
deterministic and centralized on the coordinator host; it returns the
tasks per vehicle, their estimated costs, their reservations, and the
reasons for unassigned work. A distributed solver may implement the same
interface later. A solver proposes; an acknowledged assignment commits;
the vehicle's fencing and recovery rules are independent of the solver.

### Observations revise a plan; they do not grant a clearance

An observation is typed: source identity, camera calibration, pose and
clock association, uncertainty, and evidence references. A local monitor
may request a bounded hold from an observation. A geometric planner may
propose a detour from a conservative hazard region. A deterministic
validator checks clearance, reserve and the approval envelope before a
coordinator commits the successor and keeps the completed coverage. When
an observation cannot support the decision, the affected region is marked
blocked or incomplete. A two-dimensional detection is not a
three-dimensional obstacle. The immediate control fallback never depends
on an inference call.

### Models are services around the path, behind an unprivileged port

The execution path runs from an admitted plan through the mission core,
navigation and payload execution, to the adapter. Perception,
interpretation, planning and allocation are services around that path. A
known survey template needs no model call, and a model timeout never
stalls the control loop or an urgent fallback. The model port
([ADR-0042](0042-agent-client-module-and-model-port.md)) gains separate
typed operations: instruction interpretation, which it has; observation
assessment; and plan proposal, each over the read-only context of
ADR-0047. Constrained decoding reduces format errors and establishes
neither feasibility nor authority; decoding, grounding, geometry,
capability and authorization checks stay independent, with `Unable` and
`NeedsInput` outcomes. The rule that every numeric slot of a direct
instruction appears in the operator's words stays for instruction
interpretation; a generated plan has its own provenance check against
validated geometry and data references. Deterministic admission is
followed by runtime monitoring with trusted invariant templates that are
independent of any model-generated specification. A model provider is
replaceable, and no model class is tied to a level of autonomy.

### Instructions in a person's words become drafts

A person or an agent can give the chain of command an instruction in
words. The shared client library (`pilotage-client-session`,
`mission::instruction`) makes the request to the instruction interpreter
and checks its reply. The App and the command-line operator use the same
library, so a person and an agent get the same result from the same
words.

The actions are data in an action catalog (`mission::instruction::catalog`).
Each action has these items:

- A name and a description in words.
- Typed parameters: a member lane, an amount, or an area reference.
- The lane state that offers the action.
- The effect of the action: the lane acts at once, or the lane drafts the
  change and it waits for an approval.

The library makes the interpreter request from the catalog and from the
lanes that the client follows. An instruction becomes one or more steps.
Each step is one action for one lane, in the order of the words. To add an
action, add its descriptor and the function that builds its command. The
descriptors serialize in the form that a lane can publish later, or that a
tool server can give to an agent.

The catalog does not contain approve, start or abort. A drafted change
does not apply at once. The library refuses these replies:

- A step that names a lane that the client does not follow. The library
  does not send the step to the lead above that lane.
- A move that names a member that the lane does not lead.
- A move whose amount is not in the words of the person. When the words
  state more than one amount, each amount applies only to the member that
  it is nearest to. If the words do not tie one amount to the member, the
  person must state it again.
- A step that the lane does not offer in its current state.

An area reference is the name of a place or of an area that a person drew.
The client finds the area on its map before the step becomes a command.
The interpreter does not give coordinates. The client shows each step in
fixed words that the library makes from the step, and it sends the steps
only when a person or an agent confirms them. The client sends one step
at a time. It sends a step only after the lane accepts the step before
it. If a lane refuses a step, the client does not send the steps after
it. A drafted change then waits for an approval, as other drafts do.

### Capability, authority, resources and conditions are separate axes

A vehicle's verified capability, the authority delegated to a principal,
the runtime resources of an endpoint, and the validated operating
conditions are four axes. No single grade or label collapses them. A
deployment profile may name a bundle of requirements to select; it grants
no power. Autonomy labels in a client are summaries of permissions and
verified capabilities. Onboard execution starts only on a platform that
exposes a verified companion interface; placing a computer beside an
aircraft does not establish a command path or remove its remote
controller. An embedded executor reuses the deterministic contracts with
bounded resources and platform input and output, without the desktop
host's USB, browser, or model-serving stack.

### Component ownership

| Component | Owns | Does not own |
|---|---|---|
| `pilotage-mission-core` | Plans, tasks, conditions, successor admission at a checkpoint, preemption transitions and `Uncertain`, lifecycle, run records and evidence events | Navigate, planning, transport, authority |
| `pilotage-mission` | Survey decomposition and estimates, task-to-plan binding, guidance, execution strategies | Task transitions, authority, transport |
| Planning library | Navigation search, route drafts, vehicle profiles, coordinated drafts, proposal compilation shared by clients and the host | Authority, execution, admission |
| `pilotage-protocol` | Objective, proposal, approval, plan, run and status records; intents and receipts | Execution |
| `pilotage-authority` | The mission scope, grants with envelopes and expiry, delegation that only narrows | Mission content |
| Session host | The lane at the control endpoint, bootstrap from the environment, admission and its serialization, status publication, recovery as an authorized behaviour | Planning algorithms, composition |
| Coordinator role | Team proposals, allocation, participant admission, fencing of its predecessor, journal aggregation | Vehicle control, member recovery rules |
| Clients (Sokoly) | Composition, approval, display | Independent planning logic or authority |
| Agent module | Instruction interpretation, observation assessment and plan proposal as intents | Writing to a lane or an adapter |
| Turret | Payload capabilities, pointing and zoom | Survey policy |
| Indicate | Presentation | Records |
| Adapters | Device semantics, verified limitations, execution strategies, link-loss behaviour, recovery actions | Survey policy |

## Consequences

- New wire records: objective, proposal, approval, plan, run and status;
  intents and receipts; a mission scope with grants that carry an envelope
  and an expiry.
- The mission principal gains the lane with `Idle` and `Uncertain` states;
  the environment-variable mission becomes one source of a plan.
- The survey planner emits one task per line and one for the way home,
  each with its acquisition evidence; the flight handler binds a plan per
  task.
- The planning draft becomes the shared planning library: its drafts gain
  a compilation path to proposals, its revision joins the identity rules
  of ADR-0049, and its vehicle profile gains the resource and capability
  references admission needs.
- Adapters declare execution strategies, link-loss behaviour, return and
  land actions, battery and landed state as capabilities; the DJI adapter
  advertises a host-guided strategy only, and nothing for unattended
  recovery until it is flight-verified.
- The evaluation harness grows by task type: refusal quality, task
  completion, stale-context rejection, manual override, partition
  behaviour, and end-to-end latency. Deterministic templates, typed
  judgment models and constrained generative planners are compared on
  separate suites; schema validity and task quality are separate measures.
- Sequencing, each step flyable on its own: reliable status and intents;
  proposal and explicit approval for one DJI survey; task execution with
  acquisition evidence; coordinated assignments; bounded offline revision.
  The planning draft lands in the same order: navigation search and route
  drafts first, then the proposal and approval handoff with a thin
  composer, then allocation once one survey has an observed completion
  contract. Flight verification of each step precedes the next.

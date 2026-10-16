import {translator} from './i18n.mjs';
import {errorText} from './transport.mjs';
const statuses = ['active','paused','completed','cancelled'];
const kinds = ['goal','requirement','prohibition','completion_criterion','open_question'];
const states = ['reported','candidate','withdrawn'];
const modes = ['Observed','Said','Acted','Inferred','Draft'];
const verifications = ['passed','failed','inconclusive','not_performed'];
const validSource = value => ['caller_reported','tool_reported','model_asserted'].includes(value?.kind);
const strings = value => Array.isArray(value) && value.every(item => typeof item === 'string');
export function validTaskWorkspaces(value) {
  return value?.read_only === true && value.limit === 100 && typeof value.has_more === 'boolean'
    && strings(value.namespaces) && value.namespaces.length <= 100
    && new Set(value.namespaces).size === value.namespaces.length
    && value.namespaces.every(namespace=>namespace.startsWith('project/') && namespace.length > 8);
}
export function validTask(task) {
  return !!task && typeof task.namespace === 'string' && typeof task.task_id === 'string'
    && Number.isSafeInteger(task.version) && task.version > 0 && typeof task.title === 'string'
    && validSource(task.source) && statuses.includes(task.status) && Array.isArray(task.requirements)
    && task.requirements.every(item => item && typeof item.requirement_id === 'string'
      && typeof item.text === 'string' && kinds.includes(item.kind) && states.includes(item.state));
}
export function validTaskList(value) {
  return value?.read_only === true && value.limit === 100 && Array.isArray(value.tasks)
    && value.tasks.length <= value.limit && value.tasks.every(validTask);
}
export function validTaskSnapshot(value) {
  if (value?.read_only !== true || !validTask(value.snapshot?.task)) return false;
  const checkpoint = value.snapshot.checkpoint;
  return checkpoint === null || !!checkpoint && Number.isSafeInteger(checkpoint.task_version)
    && Number.isSafeInteger(checkpoint.sequence) && typeof checkpoint.recorded_at === 'string'
    && Number.isFinite(Date.parse(checkpoint.recorded_at)) && validSource(checkpoint.source)
    && Array.isArray(checkpoint.artifacts) && checkpoint.artifacts.every(item=>item && typeof item.uri==='string' && (item.digest==null || typeof item.digest==='string'))
    && checkpoint.namespace === value.snapshot.task.namespace && checkpoint.task_id === value.snapshot.task.task_id
    && strings(checkpoint.blockers) && strings(checkpoint.next_steps) && Array.isArray(checkpoint.steps)
    && checkpoint.steps.every(step => step && typeof step.text === 'string' && modes.includes(step.mode)
      && verifications.includes(step.verification) && strings(step.evidence_refs));
}
export function validExperienceUses(value) {
  return value?.read_only === true && value.limit === 20 && Array.isArray(value.uses) && value.uses.length <= 20
    && value.uses.every(view => {
      const record=view?.record, item=record?.observation;
      return !!item && typeof view.currently_reusable === 'boolean' && strings(view.invalidation_reasons)
        && record.verification_basis === 'caller_reported_not_independently_verified_no_causal_attribution'
        && typeof record.recorded_at === 'string' && Number.isFinite(Date.parse(record.recorded_at))
        && ['namespace','task_id','use_id','candidate_id','reason','evaluation_method'].every(key=>typeof item[key]==='string')
        && Number.isSafeInteger(item.task_version) && item.task_version > 0
        && Number.isSafeInteger(item.candidate_version) && item.candidate_version > 0
        && ['proposed','adopted','rejected'].includes(item.decision) && verifications.includes(item.verification)
        && (item.related_use_id==null || typeof item.related_use_id==='string')
        && strings(item.action_event_refs) && strings(item.outcome_event_refs) && strings(item.limitations)
        && Array.isArray(item.conditions) && item.conditions.every(condition=>condition && typeof condition.condition==='string'
          && ['caller_reported_satisfied','caller_reported_not_satisfied','unknown'].includes(condition.assessment));
    });
}
export function taskPresentation(snapshot) {
  const task = snapshot.task, checkpoint = snapshot.checkpoint;
  const reported = task.requirements.filter(item => item.state === 'reported');
  return {goals:reported.filter(item => item.kind === 'goal').map(item => item.text),
    requirements:reported.filter(item => item.kind !== 'goal'),
    pending:task.requirements.filter(item => item.state === 'candidate').length,
    checkpointStale:!!checkpoint && checkpoint.task_version !== task.version};
}
const $ = id => document.getElementById(id);
function fields(target, rows) {
  target.replaceChildren(...rows.map(([label,value]) => {
    const div=document.createElement('div'), dt=document.createElement('dt'), dd=document.createElement('dd');
    dt.textContent=label; dd.textContent=value; div.append(dt,dd); return div;
  }));
}
let signature = '', optionsSignature = '', workspaceSignature = '';
export function renderTasks(state) {
  const t=translator(state.locale), available=state.metadata?.tasks_available === true;
  $('task-panel').hidden=!available;
  // Lock/instance changes clear hidden content as well as the visible surface.
  const next=JSON.stringify([state.locale,available,state.taskNamespace,state.taskWorkspaces,state.taskWorkspaceError,state.tasks,state.taskId,state.taskSnapshot,state.taskUses,state.taskUseError,state.taskError,state.taskBusy]);
  if(next===signature)return; signature=next;
  const workspaces=available ? state.taskWorkspaces : null;
  const workspaceKey=JSON.stringify(workspaces?.namespaces ?? []);
  if(workspaceKey!==workspaceSignature){
    workspaceSignature=workspaceKey;
    $('task-workspaces').replaceChildren(...(workspaces?.namespaces ?? []).map(namespace=>{
      const option=document.createElement('option');option.value=namespace;return option;
    }));
  }
  $('task-workspace-note').textContent=state.taskWorkspaceError ? t('taskWorkspaceUnavailable')
    : workspaces ? t(workspaces.has_more ? 'taskWorkspaceWindow' : 'taskWorkspaceBrowse') : '';
  $('task-load').disabled=!available || state.taskBusy;
  $('task-select').disabled=!available || !state.tasks?.length;
  const optionKey=JSON.stringify([state.locale,state.tasks,state.taskId]);
  if(optionKey!==optionsSignature){
    optionsSignature=optionKey;
    const placeholder=document.createElement('option'); placeholder.value=''; placeholder.textContent=t('taskChoose');
    const tasks=[...(state.tasks ?? [])];
    if(state.taskId && !tasks.some(task=>task.task_id===state.taskId))tasks.push({task_id:state.taskId,title:state.taskId});
    $('task-select').replaceChildren(placeholder,...tasks.map(task=>{
      const option=document.createElement('option'); option.value=task.task_id;
      option.textContent=task.status ? `${task.title} · ${t(`taskStatus_${task.status}`)}` : task.title; return option;
    }));
    $('task-select').value=state.taskId;
  }
  const message=state.taskError ? t('requestError',{resource:t('tasks'),reason:errorText(state.taskError,t)})
    : state.taskBusy ? t('loading') : !state.taskNamespace ? t('taskEnterWorkspace')
    : state.tasks?.length === 0 ? t('taskEmpty') : state.tasks?.length === 100 ? t('taskWindow')
    : !state.taskId ? t('taskChoose') : '';
  $('task-message').textContent=message;
  const snapshot=state.taskSnapshot;
  $('task-content').hidden=!snapshot;
  if(!snapshot) {
    $('task-use-note').textContent='';
    for(const id of ['task-fields','task-requirements','task-checkpoint-fields','task-steps','task-uses'])$(id).replaceChildren();
    return;
  }
  renderExperienceUses(state,t);
  const task=snapshot.task, checkpoint=snapshot.checkpoint, view=taskPresentation(snapshot);
  fields($('task-fields'),[
    [t('taskGoal'),view.goals.join('\n') || t('none')],
    [t('status'),t(`taskStatus_${task.status}`)],
    [t('taskVersion'),String(task.version)],
    [t('taskPending'),String(view.pending)],
    [t('taskBasis'),`${t(`taskSource_${task.source.kind}`)} · ${t('taskNotAuthorization')}`]
  ]);
  $('task-requirement-heading').textContent=t('taskRequirements',{count:view.requirements.length});
  $('task-requirements').replaceChildren(...view.requirements.map(item=>{
    const li=document.createElement('li');li.textContent=`${t(`taskKind_${item.kind}`)} · ${item.text}`;return li;
  }));
  $('task-checkpoint').hidden=!checkpoint;
  if(checkpoint) {
    fields($('task-checkpoint-fields'),[
      [t('taskBasis'),`${t(`taskSource_${checkpoint.source.kind}`)} · ${t('taskNotAuthorization')}`],
      [t('taskVersion'),`${checkpoint.task_version} · ${t(view.checkpointStale ? 'taskCheckpointStale' : 'taskCheckpointRecorded')}`],
      [t('time'),new Date(checkpoint.recorded_at).toLocaleString(state.locale,{hour12:false})],
      [t('taskBlockers'),checkpoint.blockers.join('\n') || t('none')],
      [t('taskNext'),checkpoint.next_steps.join('\n') || t('none')],
      [t('taskEnvironment'),checkpoint.environment || t('none')],
      [t('taskRepository'),checkpoint.repository_revision || t('none')],
      [t('taskArtifacts'),[t('taskNotRechecked'),...(checkpoint.artifacts ?? []).map(item=>`${item.uri}${item.digest ? ` · ${item.digest}` : ''}`)].join('\n')]
    ]);
    $('task-steps').replaceChildren(...checkpoint.steps.map(step=>{
      const li=document.createElement('li');
      li.textContent=`${t(`taskMode_${step.mode}`)} · ${t(`taskVerification_${step.verification}`)} · ${step.text}`;
      if(step.evidence_refs.length)li.textContent+=`\n${t('taskEvidence')}: ${step.evidence_refs.join(', ')}`;
      return li;
    }));
  } else {
    $('task-checkpoint-fields').replaceChildren(); $('task-steps').replaceChildren();
    fields($('task-fields'),[
      [t('taskGoal'),view.goals.join('\n') || t('none')], [t('status'),t(`taskStatus_${task.status}`)],
      [t('taskVersion'),String(task.version)],[t('taskPending'),String(view.pending)],
      [t('taskBasis'),`${t(`taskSource_${task.source.kind}`)} · ${t('taskNotAuthorization')}`],[t('taskCheckpoint'),t('taskNoCheckpoint')]
    ]);
  }
}


function renderExperienceUses(state,t) {
  const uses=state.taskUses;
  const expanded=new Set([...$('task-uses').querySelectorAll('details[open]')].map(node=>node.dataset.useId));
  const focused=document.activeElement?.closest?.('#task-uses details')?.dataset.useId;
  $('task-use-note').textContent=state.taskUseError ? t('requestError',{resource:t('useHeading'),reason:errorText(state.taskUseError,t)})
    : uses===null || uses===undefined ? t('loading') : !uses.length ? t('useEmpty') : t('useScope');
  $('task-uses').replaceChildren(...(uses ?? []).map(view=>{
    const record=view.record, item=record.observation;
    const li=document.createElement('li'), details=document.createElement('details'), summary=document.createElement('summary');
    details.dataset.useId=item.use_id;details.open=expanded.has(item.use_id);
    summary.textContent=`${item.candidate_id} @ ${item.candidate_version} · ${t(`useDecision_${item.decision}`)} · ${t(view.currently_reusable ? 'useReusable' : 'useUnavailable')}`;
    const dl=document.createElement('dl');dl.className='fields';
    fields(dl,[
      [t('taskBasis'),t('useBasis')],
      [t('useRecord'),item.use_id],
      [t('useRelated'),item.related_use_id || t('none')],
      [t('taskVersion'),`${item.task_version}`],
      [t('time'),new Date(record.recorded_at).toLocaleString(state.locale,{hour12:false})],
      [t('useResult'),t(`taskVerification_${item.verification}`)],
      [t('useOutcomeSources'),item.outcome_event_refs.join('\n') || t('none')],
      [t('useActionSources'),item.action_event_refs.join('\n') || t('none')],
      [t('useConditions'),item.conditions.map(condition=>`${t(`useAssessment_${condition.assessment}`)} · ${condition.condition}`).join('\n')],
      [t('useReason'),item.reason],
      [t('useMethod'),item.evaluation_method],
      [t('useLimitations'),item.limitations.join('\n')],
      [t('useCurrentCheck'),view.invalidation_reasons.map(reason=>t(`useInvalid_${reason}`)).join('\n') || t('useReusable')]
    ]);
    details.append(summary,dl);li.append(details);return li;
  }));
  if(focused)[...$('task-uses').querySelectorAll('details')].find(node=>node.dataset.useId===focused)?.querySelector('summary')?.focus({preventScroll:true});
}

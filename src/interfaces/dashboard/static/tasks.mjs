import {translator} from './i18n.mjs';
import {errorText} from './transport.mjs';
const statuses = ['active','paused','completed','cancelled'];
const kinds = ['goal','requirement','prohibition','completion_criterion','open_question'];
const states = ['reported','candidate','withdrawn'];
const modes = ['Observed','Said','Acted','Inferred','Draft'];
const verifications = ['passed','failed','inconclusive','not_performed'];
const validSource = value => ['caller_reported','tool_reported','model_asserted'].includes(value?.kind);
const strings = value => Array.isArray(value) && value.every(item => typeof item === 'string');
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
let signature = '', optionsSignature = '';
export function renderTasks(state) {
  const t=translator(state.locale), available=state.metadata?.tasks_available === true;
  $('task-panel').hidden=!available;
  // Lock/instance changes clear hidden content as well as the visible surface.
  const next=JSON.stringify([state.locale,available,state.taskNamespace,state.tasks,state.taskId,state.taskSnapshot,state.taskError,state.taskBusy]);
  if(next===signature)return; signature=next;
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
    for(const id of ['task-fields','task-requirements','task-checkpoint-fields','task-steps'])$(id).replaceChildren();
    return;
  }
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

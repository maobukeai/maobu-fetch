// Browser-only stand-in for the native retention actor. Rust time-model tests
// validate real expiry; these virtual minutes verify the React command wiring.
export function installCompletionFixture({ audit, api, read, change, publish, root, enabled,
  initiallyHeld = false, initialCompleted = 0 }) {
  const lifetime = 180_000;
  const entries = new Map();
  let interacting = initiallyHeld;
  let dismissed = false;

  const sync = () => {
    const completed = [...entries.values()].map(entry => entry.task);
    change({ completed_tasks: completed, completed_count: completed.length });
    if (enabled) {
      const hidden = dismissed || (!read().total_count && !completed.length);
      root.style.display = hidden ? 'none' : '';
      if (hidden) change({ revealed: false });
    }
    return publish();
  };
  const completedTask = (task) => ({ ...task, status: 'completed', speed_bps: 0,
    eta_seconds: null, downloaded_bytes: task.total_bytes, fetching_metadata: false,
    can_pause: false, can_resume: false });

  api.interacting = async value => {
    audit.calls.push(['interacting', value]);
    if (interacting === value) return structuredClone(read());
    interacting = value;
    return sync();
  };
  api.openCompleted = async (id, target) => {
    audit.calls.push(['open-completed', id, target]);
    const entry = entries.get(id);
    if (!entry) throw new Error('synthetic expired task');
    if (audit.holdOpen) await new Promise(resolve => { audit.releaseOpen = resolve; });
    if (audit.failOpen) {
      entry.remaining = lifetime;
      sync();
      throw new Error('Cookie: synthetic-private; token=synthetic-private');
    }
    return structuredClone(read());
  };
  audit.advance = milliseconds => {
    if (!interacting) {
      for (const [id, entry] of entries) {
        entry.remaining -= milliseconds;
        if (entry.remaining <= 0) entries.delete(id);
      }
    }
    return sync();
  };
  audit.complete = id => {
    const state = read();
    const task = state.tasks.find(item => item.id === id);
    if (!task) return sync();
    change({ tasks: state.tasks.filter(item => item.id !== id),
      total_count: Math.max(0, state.total_count - 1), total_speed_bps: 0 });
    if (!entries.has(id)) {
      const order = [[id, { task: completedTask(task), remaining: lifetime }], ...entries].slice(0, 6);
      entries.clear();
      for (const [key, value] of order) entries.set(key, value);
      dismissed = false;
    }
    return sync();
  };
  audit.addActive = (id, name = '新下载文件.bin') => {
    const state = read();
    change({ tasks: [{ id, file_name: name, status: 'downloading', total_bytes: 1048576,
      downloaded_bytes: 262144, speed_bps: 131072, eta_seconds: 6,
      fetching_metadata: false, can_pause: true, can_resume: false }, ...state.tasks].slice(0, 6),
      total_count: state.total_count + 1, total_speed_bps: 131072, summary_percent: 25 });
    dismissed = false;
    return sync();
  };
  audit.seedCompleted = count => {
    entries.clear();
    for (let index = 0; index < Math.min(count, 6); index++) {
      const id = 'completed-' + index;
      entries.set(id, { remaining: lifetime, task: completedTask({
        id, file_name: index ? '已完成文件-' + index + '.mp4' : '已完成的中文长文件名'.repeat(30) + '.mp4',
        total_bytes: 53_972_201,
      }) });
    }
    dismissed = false;
    return sync();
  };
  audit.removeCompleted = id => { entries.delete(id); return sync(); };
  Object.defineProperties(audit, {
    interacting: { get: () => interacting },
    nativeVisible: { get: () => root.style.display !== 'none' },
    completionRemaining: { get: () => [...entries].map(([id, entry]) => [id, entry.remaining]) },
  });
  const originalDismiss = api.dismiss;
  api.dismiss = async () => {
    await originalDismiss();
    entries.clear();
    dismissed = true;
    return sync();
  };
  if (initialCompleted) audit.seedCompleted(initialCompleted);
  else if (enabled) sync();
}

/**
 * 任务优先级的稳定预设值。
 *
 * 后端约定数字越小越优先；队列按数字升序排列，全局限速开启时按
 * high / normal / low 三档映射为 4 / 2 / 1 带宽权重。
 */
export const TASK_PRIORITY_PRESETS = {
  high: -1,
  normal: 0,
  low: 1,
} as const;

type QueueOrderTask = {
  id: string;
  priority: number;
  queue_position: number;
};

/**
 * 将一个任务移动到同优先级目标任务的位置，返回可直接提交给 queue_reorder 的完整 ID 顺序。
 * 跨优先级、缺失任务或原地放下均返回 null。
 */
export function reorderTaskIdsWithinPriority(
  tasks: QueueOrderTask[],
  draggedId: string,
  targetId: string,
): string[] | null {
  const dragged = tasks.find((task) => task.id === draggedId);
  const target = tasks.find((task) => task.id === targetId);
  if (!dragged || !target || dragged.priority !== target.priority) return null;

  const sorted = [...tasks].sort((left, right) =>
    left.priority - right.priority
    || left.queue_position - right.queue_position,
  );
  const fromIndex = sorted.findIndex((task) => task.id === draggedId);
  const toIndex = sorted.findIndex((task) => task.id === targetId);
  if (fromIndex === -1 || toIndex === -1 || fromIndex === toIndex) return null;

  const [moved] = sorted.splice(fromIndex, 1);
  sorted.splice(toIndex, 0, moved);
  return sorted.map((task) => task.id);
}

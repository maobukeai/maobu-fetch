/** 旧版桌面端未提供 cancel 时仅降级为暂停，并把真实结果交给界面。 */
export async function performBridgeTaskAction(id, action, signedFetch) {
  const path = `/v1/tasks/${encodeURIComponent(id)}/action`;
  const invoke = async (nextAction) => {
    const response = await signedFetch(path, { action: nextAction });
    if (!response.ok) throw new Error(await response.text() || `HTTP ${response.status}`);
    return response.json();
  };
  const result = await invoke(action);
  if (action === "cancel" && result?.success === false && /不支持的操作|unsupported action/i.test(result.error || "")) {
    return { ...(await invoke("pause")), paused_instead: true };
  }
  return result;
}

// Push notifications: shown with their buttons (each a one-time token that
// does its action without a login); a click elsewhere opens the task.
self.addEventListener("push", (e) => {
  const d = e.data ? e.data.json() : {};
  const actions = (d.actions || []).slice(0, 2);
  e.waitUntil(
    self.registration.showNotification(d.title || "reagent", {
      body: d.body || "",
      tag: d.task || undefined,
      actions: actions.map((a, i) => ({ action: `a${i}`, title: a.title })),
      data: { task: d.task, tokens: Object.fromEntries(actions.map((a, i) => [`a${i}`, a.token])) },
    }),
  );
});

self.addEventListener("notificationclick", (e) => {
  e.notification.close();
  const { task, tokens } = e.notification.data || {};
  const token = e.action && tokens?.[e.action];
  if (token) {
    e.waitUntil(
      fetch("/api/action", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ token }) }).then(async (r) => {
        const d = await r.json().catch(() => ({}));
        // Done, or not (used, expired, no longer waiting): say so.
        await self.registration.showNotification(r.ok ? `reagent: ${d.done}` : "reagent: not done", { body: r.ok ? "" : d.error || "", tag: task || undefined });
      }),
    );
    return;
  }
  e.waitUntil(self.clients.openWindow(task ? `/#/task/${task}` : "/"));
});

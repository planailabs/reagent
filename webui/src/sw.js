// Push notifications: shown, and a click opens the task.
self.addEventListener("push", (e) => {
  const d = e.data ? e.data.json() : {};
  e.waitUntil(self.registration.showNotification(d.title || "reagent", { body: d.body || "", data: { task: d.task }, tag: d.task || undefined }));
});

self.addEventListener("notificationclick", (e) => {
  e.notification.close();
  const url = e.notification.data?.task ? `/#/task/${e.notification.data.task}` : "/";
  e.waitUntil(self.clients.openWindow(url));
});

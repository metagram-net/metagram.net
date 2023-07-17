// These are the minimum requirements to implement a service worker. There's _a
// lot_ more that can be done here.

// TODO: Store the install event to trigger it later on user action.
self.addEventListener("install", (_event) => null);

// TODO: Actually cache things maybe?
self.addEventListener("fetch", (event) => {
  return event.respondWith(fetch(event.request));
});

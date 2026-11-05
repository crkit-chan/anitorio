/*! coi-serviceworker v0.1.7 - Guido Zuidhof and contributors, licensed under MIT */

interface Window {
  coi?: {
    shouldRegister?(): boolean;
    shouldDeregister?(): boolean;
    coepCredentialless?(): boolean;
    doReload?(): void;
    quiet?: boolean;
  };
}

let coepCredentialless = false;

(() => {
  if (typeof window === "undefined") {
    const sw = self as unknown as ServiceWorkerGlobalScope;

    sw.addEventListener("install", () => sw.skipWaiting());
    sw.addEventListener("activate", (event) => event.waitUntil(sw.clients.claim()));
    sw.addEventListener("message", (ev) => {
      if (!ev.data) return;
      else if (ev.data.type === "deregister") {
        sw.registration
          .unregister()
          .then(() => sw.clients.matchAll())
          .then((clients) => clients.forEach((c) => c.type === "window" && (c as WindowClient).navigate(c.url)));
      } else if (ev.data.type === "coepCredentialless") {
        coepCredentialless = ev.data.value;
      }
    });
    sw.addEventListener("fetch", function (event) {
      const r = event.request;
      if (r.cache === "only-if-cached" && r.mode !== "same-origin") return;

      const request = coepCredentialless && r.mode === "no-cors" ? new Request(r, { credentials: "omit" }) : r;

      event.respondWith(
        fetch(request)
          .then((response) => {
            if (response.status === 0) return response;

            const url = new URL(r.url);

            // StreamSaver's MITM iframe must not receive COOP.
            const isStreamSaverMitm = url.pathname.endsWith("/anitorio/streamsaver/mitm.html");
            const newHeaders = new Headers(response.headers);

            newHeaders.set("Cross-Origin-Embedder-Policy", coepCredentialless ? "credentialless" : "require-corp");

            coepCredentialless || newHeaders.set("Cross-Origin-Resource-Policy", "cross-origin");
            isStreamSaverMitm || newHeaders.set("Cross-Origin-Opener-Policy", "same-origin");

            return new Response(response.body, {
              status: response.status,
              statusText: response.statusText,
              headers: newHeaders,
            });
          })
          .catch((e) => {
            console.error(e);
            return new Response("Failed to fetch", { status: 500 });
          }),
      );
    });
  } else {
    // You can customize the behavior of this script through a global `coi` variable.
    const coi = {
      shouldRegister: () => true,
      shouldDeregister: () => false,
      coepCredentialless: () => !("chrome" in window || "netscape" in window),
      doReload: () => window.location.reload(),
      quiet: false,
      ...window.coi,
    };

    const n = navigator;
    if (n.serviceWorker && n.serviceWorker.controller) {
      n.serviceWorker.controller.postMessage({ type: "coepCredentialless", value: coi.coepCredentialless() });
      coi.shouldDeregister() && n.serviceWorker.controller.postMessage({ type: "deregister" });
    }

    // If we're already coi: do nothing. Perhaps it's due to this script doing its job, or COOP/COEP are
    // already set from the origin server. Also if the browser has no notion of crossOriginIsolated, just give up here.
    if (window.crossOriginIsolated !== false || !coi.shouldRegister()) return;
    if (!window.isSecureContext) {
      !coi.quiet && console.log("COOP/COEP Service Worker not registered, a secure context is required.");
      return;
    }

    // In some environments (e.g. Chrome incognito mode) this won't be available
    if (n.serviceWorker && window.document.currentScript instanceof HTMLScriptElement) {
      n.serviceWorker.register(window.document.currentScript.src).then(
        (registration) => {
          !coi.quiet && console.log("COOP/COEP Service Worker registered", registration.scope);

          registration.addEventListener("updatefound", () => {
            !coi.quiet && console.log("Reloading page to make use of updated COOP/COEP Service Worker.");
            coi.doReload();
          });

          // If the registration is active, but it's not controlling the page
          if (registration.active && !n.serviceWorker.controller) {
            !coi.quiet && console.log("Reloading page to make use of COOP/COEP Service Worker.");
            coi.doReload();
          }
        },
        (err) => !coi.quiet && console.error("COOP/COEP Service Worker failed to register:", err),
      );
    }
  }
})();

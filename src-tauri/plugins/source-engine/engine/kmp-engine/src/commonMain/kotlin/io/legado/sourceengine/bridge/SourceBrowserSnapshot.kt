package io.legado.sourceengine.bridge

import kotlinx.serialization.json.JsonPrimitive
import kotlin.random.Random

/** Trusted native snippets for the private offline snapshot and desktop dynamic source renderer. */
internal object SourceBrowserSnapshot {
    private const val CSP = "default-src 'none'; img-src data: blob:; style-src 'unsafe-inline' data:; " +
        "font-src data:; media-src data:; connect-src 'none'; script-src 'unsafe-eval'; object-src 'none'; " +
        "frame-src 'none'; child-src 'none'; worker-src 'none'; form-action 'none'; " +
        "base-uri http: https:; navigate-to 'none'"

    fun newCapabilityId(): String = buildString(32) {
        repeat(32) { append("0123456789abcdef"[Random.nextInt(16)]) }
    }

    /**
     * Install CSP in the initial about:blank document before parsing any page markup. A template
     * element keeps its subtree inert while we remove page code and active resource nodes; ordinary
     * href/src attributes survive so rules still get source-URL resolution from the real base URL.
     */
    fun prepareDocument(request: HostBrowserRequest): String {
        if (request.mode == HostBrowserMode.DYNAMIC_PAGE) return prepareDynamicPage(request)
        val html = jsString(request.html)
        val baseUrl = jsString(request.baseUrl)
        val csp = jsString(CSP)
        val capabilityId = jsString(request.capabilityId)
        return """(function(){
            try {
                const policy = document.createElement('meta');
                policy.httpEquiv = 'Content-Security-Policy';
                policy.content = $csp;
                document.head.prepend(policy);
                const base = document.createElement('base');
                base.href = $baseUrl;
                document.head.insertBefore(base, policy.nextSibling);
                const template = document.createElement('template');
                template.innerHTML = $html;
                const fragment = template.content;
                fragment.querySelectorAll('script, iframe, frame, frameset, object, embed, applet, base, link[href], meta[http-equiv="refresh" i], meta[http-equiv="content-security-policy" i]')
                    .forEach((node) => node.remove());
                fragment.querySelectorAll('*').forEach((element) => {
                    for (const attr of Array.from(element.attributes)) {
                        if (attr.name.toLowerCase().startsWith('on') || attr.name.toLowerCase() === 'srcdoc') {
                            element.removeAttribute(attr.name);
                        }
                    }
                    if (element.hasAttribute('style') && /url\s*\(|@import/i.test(element.getAttribute('style'))) {
                        element.removeAttribute('style');
                    }
                });
                fragment.querySelectorAll('style').forEach((style) => {
                    if (/@import|url\s*\(/i.test(style.textContent || '')) style.remove();
                });
                const sourceHead = fragment.querySelector('head');
                const sourceBody = fragment.querySelector('body');
                if (sourceHead) document.head.append(...Array.from(sourceHead.childNodes));
                if (sourceBody) document.body.replaceChildren(...Array.from(sourceBody.childNodes));
                else document.body.replaceChildren(...Array.from(fragment.childNodes));
                return JSON.stringify({ ok: true, value: { capabilityId: $capabilityId } });
            } catch (_) {
                return JSON.stringify({ ok: false, error: 'Offline source document could not be prepared' });
            }
        })()""".trimIndent()
    }

    /**
     * Evaluates the source's already-existing webJs rule. Browser APIs that would need page
     * bootstrap, network access, or site browser storage fail the operation explicitly.
     */
    fun evaluateRule(request: HostBrowserRequest): String {
        if (request.mode == HostBrowserMode.DYNAMIC_PAGE) return startDynamicRule(request)
        val script = jsString(request.javaScript.ifBlank { "document.documentElement.outerHTML" })
        val capabilityId = jsString(request.capabilityId)
        val finalUrl = jsString(request.baseUrl)
        return """(function(){
            const violations = new Set();
            const blocked = (name) => function(){
                violations.add(name);
                throw new Error('Unsupported by the offline source browser snapshot: ' + name);
            };
            const defineBlocked = (target, name, label) => {
                try { Object.defineProperty(target, name, { configurable: false, get: blocked(label) }); }
                catch (_) { try { target[name] = blocked(label); } catch (_) {} }
            };
            defineBlocked(window, 'localStorage', 'localStorage');
            defineBlocked(window, 'sessionStorage', 'sessionStorage');
            defineBlocked(document, 'cookie', 'browser cookies');
            for (const name of ['fetch', 'XMLHttpRequest', 'WebSocket', 'EventSource', 'Worker', 'SharedWorker']) {
                try { Object.defineProperty(window, name, { configurable: false, writable: false, value: blocked(name) }); }
                catch (_) { try { window[name] = blocked(name); } catch (_) {} }
            }
            try { Object.defineProperty(navigator, 'sendBeacon', { configurable: false, value: blocked('sendBeacon') }); }
            catch (_) {}
            try { Object.defineProperty(window, 'open', { configurable: false, value: blocked('window.open') }); }
            catch (_) {}
            try { Object.defineProperty(HTMLFormElement.prototype, 'submit', { configurable: false, value: blocked('form submission') }); }
            catch (_) {}
            try { Object.defineProperty(HTMLFormElement.prototype, 'requestSubmit', { configurable: false, value: blocked('form submission') }); }
            catch (_) {}
            const resourceTags = new Set(['IMG', 'VIDEO', 'AUDIO', 'SOURCE', 'TRACK', 'SCRIPT', 'IFRAME', 'FRAME', 'OBJECT', 'EMBED', 'INPUT']);
            const inspectResource = (element) => {
                if (!element || element.nodeType !== 1) return;
                const tag = element.tagName;
                for (const name of ['src', 'srcset']) {
                    const value = element.getAttribute(name);
                    if (value && resourceTags.has(tag) && !/^(data:|blob:|about:)/i.test(value.trim())) {
                        violations.add('dynamic resource request');
                    }
                }
                if (['LINK', 'IFRAME', 'FRAME', 'OBJECT', 'EMBED'].includes(tag) && element.hasAttribute('href')) {
                    violations.add('dynamic resource request');
                }
                if (element.hasAttribute('style') && /url\s*\(|@import/i.test(element.getAttribute('style'))) {
                    violations.add('dynamic resource request');
                }
            };
            const inspectMutations = (records) => records.forEach((record) => {
                if (record.type === 'attributes') inspectResource(record.target);
                record.addedNodes && Array.from(record.addedNodes).forEach((node) => {
                    inspectResource(node);
                    if (node.querySelectorAll) node.querySelectorAll('*').forEach(inspectResource);
                });
            });
            const resourceObserver = new MutationObserver(inspectMutations);
            resourceObserver.observe(document, { subtree: true, childList: true, attributes: true, attributeFilter: ['src', 'srcset', 'href', 'style'] });
            try { Object.defineProperty(window, '__legadoSnapshotViolations', { configurable: false, value: () => Array.from(violations) }); }
            catch (_) {}
            let value;
            try { value = (0, eval)($script); }
            catch (_) {
                return JSON.stringify({ ok: false, error: 'Source webJs rule failed' });
            }
            resourceObserver.disconnect();
            inspectMutations(resourceObserver.takeRecords());
            const unsupported = Array.from(violations);
            if (unsupported.length) {
                return JSON.stringify({ ok: false, error: 'Unsupported browser capability used: ' + unsupported.join(', ') });
            }
            try {
                if (value && (typeof value === 'object' || typeof value === 'function') && typeof value.then === 'function') {
                    return JSON.stringify({ ok: false, error: 'Asynchronous webJs results are unsupported by the offline source browser snapshot' });
                }
            } catch (_) {
                return JSON.stringify({ ok: false, error: 'Browser rule result could not be inspected' });
            }
            if (value === null || value === undefined) {
                return JSON.stringify({ ok: false, error: 'Browser rule returned no result' });
            }
            let body;
            try { body = typeof value === 'string' ? value : JSON.stringify(value); }
            catch (_) {
                return JSON.stringify({ ok: false, error: 'Browser rule result is not serializable' });
            }
            if (body && body.length > 4194304) {
                return JSON.stringify({ ok: false, error: 'Private browser result exceeds the 4 MiB limit' });
            }
            if (body === undefined || body.length === 0) {
                return JSON.stringify({ ok: false, error: 'Browser rule returned an empty result' });
            }
            return JSON.stringify({ ok: true, value: { capabilityId: $capabilityId, finalUrl: $finalUrl, body: body } });
        })()""".trimIndent()
    }

    private fun prepareDynamicPage(request: HostBrowserRequest): String = """(function(){
        try {
            const content = "default-src http: https: data: blob:; script-src http: https: 'unsafe-inline' 'unsafe-eval'; " +
                "style-src http: https: data: 'unsafe-inline'; img-src http: https: data: blob:; " +
                "font-src http: https: data:; media-src http: https: data: blob:; " +
                "connect-src http: https: ws: wss:; frame-src 'self'; child-src 'self'; " +
                "worker-src http: https: blob:; object-src 'none'; form-action http: https:; base-uri http: https:";
            const installPolicy = () => {
                if (!document.head) return false;
                const policy = document.createElement('meta');
                policy.httpEquiv = 'Content-Security-Policy';
                policy.content = content;
                policy.setAttribute('data-legado-private-policy', '');
                document.head.prepend(policy);
                return true;
            };
            if (!installPolicy()) {
                const observer = new MutationObserver(() => {
                    if (installPolicy()) observer.disconnect();
                });
                observer.observe(document, { childList: true, subtree: true });
            }
            try { Object.defineProperty(window, 'open', { configurable: false, value: function(){ return null; } }); }
            catch (_) { window.open = function(){ return null; }; }
            for (const name of ['__TAURI__', '__TAURI_INTERNALS__']) {
                try { delete window[name]; } catch (_) {}
                try { Object.defineProperty(window, name, { configurable: false, value: undefined }); } catch (_) {}
            }
        } catch (_) {}
    })()""".trimIndent()

    private fun startDynamicRule(request: HostBrowserRequest): String {
        val script = jsString(request.javaScript.ifBlank { "document.documentElement.outerHTML" })
        val capabilityId = jsString(request.capabilityId)
        val resultKey = jsString("__legadoDynamicResult_${request.capabilityId}")
        return """(function(){
            const key = $resultKey;
            const capabilityId = $capabilityId;
            const holder = Object.create(null);
            try {
                Object.defineProperty(window, key, { configurable: false, enumerable: false, value: holder });
            } catch (_) {
                return JSON.stringify({ ok: false, error: 'Private dynamic browser result state is unavailable' });
            }
            const finish = (value, error) => {
                if (holder.done) return;
                holder.done = true;
                if (error) {
                    holder.json = JSON.stringify({ ok: false, error: 'Source webJs rule failed: ' + String(error).slice(0, 300) });
                    return;
                }
                if (value === null || value === undefined) {
                    holder.json = JSON.stringify({ ok: false, error: 'Browser rule returned no result' });
                    return;
                }
                let body;
                try { body = typeof value === 'string' ? value : JSON.stringify(value); }
                catch (_) {
                    holder.json = JSON.stringify({ ok: false, error: 'Browser rule result is not serializable' });
                    return;
                }
                if (body === undefined || body.length === 0) {
                    holder.json = JSON.stringify({ ok: false, error: 'Browser rule returned an empty result' });
                    return;
                }
                if (body.length > 4194304) {
                    holder.json = JSON.stringify({ ok: false, error: 'Private browser result exceeds the 4 MiB limit' });
                    return;
                }
                holder.json = JSON.stringify({ ok: true, value: {
                    capabilityId: capabilityId,
                    finalUrl: window.location.href,
                    body: body
                } });
            };
            let value;
            try { value = (0, eval)($script); }
            catch (error) { finish(undefined, error); return JSON.stringify({ ok: true, value: { started: true } }); }
            const timeout = new Promise((_, reject) => setTimeout(() => reject(new Error('webJs promise timed out')), 15000));
            Promise.race([Promise.resolve(value), timeout]).then(
                result => finish(result, null),
                error => finish(undefined, error)
            );
            return JSON.stringify({ ok: true, value: { started: true } });
        })()""".trimIndent()
    }

    fun jsString(value: String): String = JsonPrimitive(value).toString()
}

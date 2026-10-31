package io.legado.app.model.script

/** 类入口与 JVM 使用同一个 JsBootstrap；只有底层分派方式不同。 */
internal const val NATIVE_JAVA_BOOTSTRAP = """
function __getDangerousApi() { return false; }
function __loadJavaClass(name) { return __nativeDispatch(0, 3400, [name]); }
function __classExists(name) { return __loadJavaClass(name) > 0; }
function __isInterface(handle) { return __nativeDispatch(0, 3406, [handle]); }
function __newJavaInstance(handle, args) {
    var value = __nativeDispatch(0, 3401, [handle, args]);
    return typeof value === 'string' ? Object(value) : value;
}
function __callStaticMethod(handle, method, args) { return __nativeDispatch(0, 3402, [handle, method, args]); }
function __getStaticField(handle, name) { return __nativeDispatch(0, 3404, [handle, name]); }
function __setStaticField() { throw new Error('Java static fields are read-only on native'); }
function __registerJsFunctionNative(name) { return name; }
function __newJavaAdapter(handle, name) {
    var implementation = globalThis[name];
    return __nativeDispatch(0, 3405, [handle, function(method, args) {
        var fn = typeof implementation === 'function' ? implementation : implementation[method];
        if (typeof fn !== 'function') throw new Error('Missing JavaAdapter method: ' + method);
        return fn.apply(implementation, args);
    }]);
}

function __createJavaCompatObj(handle, collection) {
    if (!handle) return null;
    function call(method, args) { return __nativeDispatch(handle, 3403, [method, args]); }
    return new Proxy({}, { get: function(obj, key) {
        if (key === '__javaCompatHandle__') return handle;
        if (key === 'then' || key === '__classHandle__' || key === '__pkgPath__') return undefined;
        if (key === Symbol.toPrimitive) return function() { return call('toString', []); };
        if (key === 'nextBytes') return function(bytes) {
            var random = call('generateSeed', [bytes.length]);
            for (var i = 0; i < bytes.length; i++) bytes[i] = (random[i] << 24) >> 24;
        };
        if (collection && key === 'length') return call('size', []);
        if (collection && key === Symbol.iterator) return function*() {
            for (var i = 0; i < call('size', []); i++) yield call('get', [i]);
        };
        if (collection && /^(0|[1-9][0-9]*)$/.test(String(key))) return call('get', [Number(key)]);
        if (typeof key !== 'string') return undefined;
        return function() { return call(key, Array.prototype.slice.call(arguments)); };
    }});
}
"""

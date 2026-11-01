package io.legado.app.model.script

internal const val NATIVE_CRYPTO_BOOTSTRAP = """
function __createCryptoObj(handle) {
    var obj = {};
    obj.encryptBase64 = function(data, charset) { return __nativeDispatch(handle, 1001, arguments.length > 1 ? [data, charset] : [data]); };
    obj.decryptStr = function(data) { return __nativeDispatch(handle, 1002, [data]); };
    obj.decrypt = function(data) { return __nativeDispatch(handle, 1003, [data]); };
    obj.setIv = function(iv) { __nativeDispatch(handle, 1004, [iv]); return obj; };
    obj.encrypt = function(data) { return __nativeDispatch(handle, 1005, [data]); };
    obj.encryptHex = function(data) { return __nativeDispatch(handle, 1006, [data]); };
    // @@methods:__createCryptoObj@@
    return obj;
}

function __createAsymCryptoObj(handle) {
    var obj = {};
    obj.setPrivateKey = function(key) { __nativeDispatch(handle, 1101, [key]); return obj; };
    obj.setPublicKey = function(key) { __nativeDispatch(handle, 1102, [key]); return obj; };
    obj.decrypt = function(data, usePublicKey = true) { return __nativeDispatch(handle, 1103, [data, usePublicKey]); };
    obj.encrypt = function(data, usePublicKey = true) { return __nativeDispatch(handle, 1104, [data, usePublicKey]); };
    obj.decryptStr = function(data, usePublicKey = true) { return __nativeDispatch(handle, 1105, [data, usePublicKey]); };
    obj.encryptHex = function(data, usePublicKey = true) { return __nativeDispatch(handle, 1106, [data, usePublicKey]); };
    obj.encryptBase64 = function(data, usePublicKey = true) { return __nativeDispatch(handle, 1107, [data, usePublicKey]); };
    // @@methods:__createAsymCryptoObj@@
    return obj;
}

function __createSignObj(handle) {
    var obj = {};
    obj.setPrivateKey = function(key) { __nativeDispatch(handle, 1201, [key]); return obj; };
    obj.setPublicKey = function(key) { __nativeDispatch(handle, 1202, [key]); return obj; };
    obj.sign = function(data) { return __nativeDispatch(handle, 1203, [data]); };
    obj.signHex = function(data) { return __nativeDispatch(handle, 1204, [data]); };
    obj.verify = function(data, sign) { return __nativeDispatch(handle, 1205, [data, sign]); };
    // @@methods:__createSignObj@@
    return obj;
}
"""

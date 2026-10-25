package io.legado.app.help.source

import kotlin.concurrent.Volatile

/** 核心只声明存储能力，命名空间保持既有持久化格式。 */
enum class SourceStorageNamespace(val value:String){
    Cache("source-cache"),Cookies("cookies"),FileCache("file-cache"),PersistentFileCache("persistent-file-cache"),
    ExploreKinds("explore-kinds"),BookVariables("book-variables")
}
interface SourceStorage {
    fun get(namespace:SourceStorageNamespace,key:String):String?
    fun put(namespace:SourceStorageNamespace,key:String,value:String,ttlSeconds:Long=0)
    fun delete(namespace:SourceStorageNamespace,key:String)
    fun deletePrefixes(namespace:SourceStorageNamespace,prefixes:List<String>)
    fun deleteContains(namespace:SourceStorageNamespace,value:String)
    fun clear(namespace:SourceStorageNamespace)
}
object SourceStorageProviders {
    @Volatile var impl:SourceStorage?=null
    fun get():SourceStorage=impl?:error("宿主未提供源存储能力")
}

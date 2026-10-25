//! 源业务：定义解析、目录快照、版本与登录；持久化只调用数据库接口。
mod catalog;
pub(crate) mod import;
pub(crate) mod login;
pub(crate) use catalog::{content_type, Catalog, SourceRecord};
#[cfg(test)]
mod tests;

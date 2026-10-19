//! 书籍加入、信息刷新和书籍详情资源。

use super::*;

impl ApplicationService {
    pub async fn get_book(&self, book_id: &str) -> Result<Value, String> {
        let _operation = self.operation_read().await;
        validate_id(book_id, "bookId")?;
        let reference = self.store.book_ref(book_id).map_err(|error| error.to_string())?;
        if !self.store.regular_file_exists(&reference).await.map_err(|error| error.to_string())? {
            return Err("Book is no longer available".into());
        }
        Ok(self.resource_descriptor(&reference))
    }
}

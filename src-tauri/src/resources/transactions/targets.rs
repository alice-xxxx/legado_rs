//! 构造并校验事务目标与清理目标。

use super::*;

impl Replacement {
    pub(crate) fn public_bytes(reference: ResourceRef, bytes: Vec<u8>) -> Result<Self, TransactionError> {
        let target = TransactionTarget::public_resource(reference)?;
        let TransactionTarget::PublicJson(path) = &target;
        if path.ends_with(".json") { validate_public_json_bytes(&bytes)?; }
        Ok(Self { target, bytes: Some(bytes) })
    }
    pub(crate) fn public_json(
        reference: ResourceRef,
        bytes: Vec<u8>,
    ) -> Result<Self, TransactionError> {
        let target = TransactionTarget::public_json(reference)?;
        validate_public_json_bytes(&bytes)?;
        Ok(Self {
            target,
            bytes: Some(bytes),
        })
    }


}

impl PostCommitDelete {
    pub(crate) fn chapter(reference: ResourceRef) -> Result<Self, TransactionError> {
        if !reference.is_local() || !is_chapter_resource_path(reference.path()) {
            return Err(TransactionError::new(
                "Post-commit cleanup accepts only typed chapter resources",
            ));
        }
        ResourceRef::new(format!("resource://{}", reference.path()))
            .map_err(|_| TransactionError::new("Invalid chapter cleanup path"))?;
        Ok(Self {
            path: reference.path().to_owned(),
        })
    }

}

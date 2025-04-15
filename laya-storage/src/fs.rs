use std::fs::File;
use std::path::Path;
use std::str::FromStr;

use mediatype::MediaTypeBuf;

use crate::{FileOrStream, StorageError, StorageObject, StorageProvider};

pub struct FilesystemStorageProvider {
    base: Box<Path>,
}

impl StorageProvider for FilesystemStorageProvider {
    fn open<'a, 'fut>(
        &'a self,
        id: &str,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<crate::StorageObject, crate::StorageError>> + Send + 'fut>>
    where
        'a: 'fut,
    {
        let id = id.to_string();

        Box::pin(async move {
            let path = self.base.join(&id);

            if !std::fs::exists(&path)? {
                return Err(StorageError::NotFound);
            }

            let file = File::open(&path)?;
            let metadata = file.metadata()?;
            let last_modified = metadata.modified().ok();
            let media_type = mime_guess::from_path(&path)
                .first()
                .and_then(|mime| MediaTypeBuf::from_str(mime.essence_str()).ok())
                .ok_or(StorageError::UnknownFormat)?;

            Ok(StorageObject {
                content: FileOrStream::File(path.into_boxed_path()),
                last_modified,
                media_type,
                name: Some(id),
            })
        })
    }
}

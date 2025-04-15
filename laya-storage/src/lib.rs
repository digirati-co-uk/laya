use std::error::Error;
use std::fmt::Display;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::time::SystemTime;

use futures::{AsyncRead, AsyncSeek};
use mediatype::MediaTypeBuf;

mod fs;
pub use fs::FilesystemStorageProvider;

pub type FileStreamProvider = Box<dyn FnOnce(&Path) -> Box<dyn SeekableStream> + Send>;

pub trait SeekableStream: AsyncRead + AsyncSeek {}
impl<T: AsyncRead + AsyncSeek> SeekableStream for T {}

/// An object stored by a storage provider.
///
/// This structure represents a stored object with optional metadata
/// such as a name and last modification time, along with its contents.
pub struct StorageObject {
    /// A handle to the contents of this object.
    pub content: FileOrStream,

    /// An optional date representing the last time this object was modified, allowing downstream
    /// clients to cache the result.
    pub last_modified: Option<SystemTime>,

    /// The media type of this storage object.
    pub media_type: MediaTypeBuf,

    /// An optional name associated with the storage object.
    pub name: Option<String>,
}

/// Provides storage for data identified by unique string identifiers.
pub trait StorageProvider: Send + Sync {
    /// Opens a handle to the random-access storage identified by the unique id `id`.
    /// The storage may point to an asynchronous stream, or a locally available file.
    fn open<'a, 'fut>(
        &'a self,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<StorageObject, StorageError>> + Send + 'fut>>
    where
        'a: 'fut;

    /// Check if this storage provider is ready to begin/still able to service
    /// [StorageProvider::open] requests.
    fn healthcheck(&self) -> Pin<Box<dyn Future<Output = Result<(), StorageError>> + Send>> {
        Box::pin(futures::future::ready(Ok(())))
    }
}

/// A path to a file encapsulated with a factory method that can provide an asynchronous stream if
/// necessary.
pub struct FileStream {
    pub path: Box<Path>,
    pub stream_factory: FileStreamProvider,
}

/// A source of data, coming from a local file or asynchronous stream.
///
/// This enum represents the different data can be loaded.
///
/// The reason [FileOrStream::File] and [FileOrStream::Stream] are separate despite [SeekableStream]
/// being able to represent a file is so decoders can decide to make optimizations
/// based on direct filesystem access, such as using memory mapped files or
/// DMA.
pub enum FileOrStream {
    /// Storage represented by a file on the filesystem.
    File(Box<Path>),

    /// Storage represented by a seekable stream.
    Stream(Box<dyn SeekableStream + Send>),
}

impl From<std::io::Error> for StorageError {
    fn from(value: std::io::Error) -> Self {
        StorageError::Internal(Box::from(value))
    }
}

#[derive(Debug)]
pub enum StorageError {
    /// The [StorageProvider] implementation lacks the credentials to access the given object.
    AccessDenied,

    /// An internal and irrecoverable storage adapter error occurred.
    Internal(Box<dyn Error + Send + Sync>),

    /// A [StorageObject] with the given identifier could not be found in the storage.
    NotFound,

    /// Generic storage adapter implementation errors.
    Other(String),

    /// A [StorageObject] was found in storage, but the type of object could not be identified.
    UnknownFormat,
}

impl Error for StorageError {}

impl Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::AccessDenied => write!(f, "access was denied"),
            StorageError::NotFound => write!(f, "data could not be found"),
            StorageError::Other(reason) => write!(f, "other: {reason}"),
            StorageError::Internal(e) => write!(f, "{}", e),
            StorageError::UnknownFormat => {
                write!(f, "the file format of the storage object is unknown")
            }
        }
    }
}

impl<T: StorageProvider> StorageProvider for Box<T> {
    fn open<'a, 'fut>(
        &'a self,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<StorageObject, StorageError>> + Send + 'fut>>
    where
        'a: 'fut,
    {
        T::open(self, id)
    }
}

impl StorageProvider for Box<dyn StorageProvider> {
    fn open<'a, 'fut>(
        &'a self,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<StorageObject, StorageError>> + Send + 'fut>>
    where
        'a: 'fut,
    {
        <dyn StorageProvider>::open(self, id)
    }
}

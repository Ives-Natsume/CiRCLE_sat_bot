use anyhow::Ok;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot}
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use crate::i18n;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileFormat {
    Json,
    Text,
    Png,
    Toml,
}

pub enum FileData {
    Json(serde_json::Value),
    Text(String),
    Png(Vec<u8>),
    Toml(toml::Value),
}

impl FileFormat {
    pub fn to_extension(&self) -> &'static str {
        match self {
            FileFormat::Json => "json",
            FileFormat::Png => "png",
            FileFormat::Text => "txt",
            FileFormat::Toml => "toml",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "json" => Some(FileFormat::Json),
            "png" => Some(FileFormat::Png),
            "txt" => Some(FileFormat::Text),
            "toml" => Some(FileFormat::Toml),
            _ => None,
        }
    }
}

pub enum FileRequest {
    Read {
        path: PathBuf,
        format: FileFormat,
        responder: oneshot::Sender<Result<FileData, anyhow::Error>>,
    },
    Write {
        path: PathBuf,
        format: FileFormat,
        data: FileData,
        responder: oneshot::Sender<Result<(), anyhow::Error>>,
    },
    Exists {
        path: PathBuf,
        responder: oneshot::Sender<Result<bool, anyhow::Error>>,
    },
    Delete {
        path: PathBuf,
        responder: oneshot::Sender<Result<(), anyhow::Error>>,
    },
}

pub async fn file_manager(mut rx: mpsc::Receiver<FileRequest>) {
    while let Some(request) = rx.recv().await {
        match request {
            FileRequest::Read { path, format, responder } => {
                let result = async {
                    let mut file = File::open(&path).await?;
                    let mut content = Vec::new();
                    file.read_to_end(&mut content).await?;

                    match format {
                        FileFormat::Json => Ok(FileData::Json(
                            serde_json::from_slice(&content)
                                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
                        )),
                        FileFormat::Text => Ok(FileData::Text(String::from_utf8(content)
                            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?)),
                        FileFormat::Png => Ok(FileData::Png(content)),
                        FileFormat::Toml => Ok(FileData::Toml(
                            toml::from_slice(&content)
                                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
                        )),
                    }
                }.await;

                if let Err(ref e) = result {
                    tracing::error!("{} {}: {}", i18n::text("file_read_error"), path.display(), e);
                }
                let _ = responder.send(result);
            }
            FileRequest::Write { path, data, responder, .. } => {
                let result = async {
                    let mut file = File::create(&path)
                        .await
                        .map_err(|e| anyhow::Error::new(e))?;
                    match data {
                        FileData::Json(json_data) => {
                            let json_str = serde_json::to_string(&json_data)
                                .map_err(|e| anyhow::Error::new(e))?;
                            file.write_all(json_str.as_bytes()).await?;
                        },
                        FileData::Text(text) => {
                            file.write_all(text.as_bytes()).await?;
                        },
                        FileData::Png(png_data) => {
                            file.write_all(&png_data).await?;
                        },
                        FileData::Toml(toml_data) => {
                            let toml_str = toml::to_string(&toml_data)
                                .map_err(|e| anyhow::Error::new(e))?;
                            file.write_all(toml_str.as_bytes()).await?;
                        },
                    }
                    Ok(())
                }.await;

                if let Err(ref e) = result {
                    tracing::error!("{} {}: {}", i18n::text("file_write_error"), path.display(), e);
                }
                let _ = responder.send(result);
            }
            FileRequest::Exists { path, responder } => {
                let result = async {
                    let metadata = tokio::fs::metadata(&path).await?;
                    Ok(metadata.is_file())
                }.await;

                if let Err(ref e) = result {
                    tracing::error!("{} {}: {}", i18n::text("file_exists_error"), path.display(), e);
                }
                let _ = responder.send(result);
            }
            FileRequest::Delete { path, responder } => {
                let result = async {
                    tokio::fs::remove_file(&path).await?;
                    Ok(())
                }.await;

                if let Err(ref e) = result {
                    tracing::error!("{} {}: {}", i18n::text("file_delete_error"), path.display(), e);
                }
                let _ = responder.send(result);
            }
        }
    }
}
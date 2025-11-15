#![allow(unused)]
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot, RwLock},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::path::PathBuf;
use crate::i18n;
use crate::config::{Config, CONFIG_PATH};

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
                    if let Some(parent) = path.parent() {
                        if !parent.exists() {
                            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                                anyhow::anyhow!(
                                    "Failed to create directory {}: {}",
                                    parent.display(),
                                    e
                                )
                            })?;
                        }
                    }
                    let mut file = File::create(&path)
                        .await
                        .map_err(|e| anyhow::Error::new(e))?;
                    match data {
                        FileData::Json(json_data) => {
                            let json_str = serde_json::to_string_pretty(&json_data)
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

pub async fn read_config(
    tx_filerequest: Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>,
) -> anyhow::Result<Config> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let request = FileRequest::Read {
        path: CONFIG_PATH.into(),
        format: FileFormat::Json,
        responder: tx,
    };

    let tx_filerequest = tx_filerequest.write().await;
    if let Err(e) = tx_filerequest.send(request).await {
        return Err(anyhow::anyhow!("Failed to send file read request: {}", e));
    }

    let file_result = match rx.await {
        Ok(result) => result,
        Err(e) => return Err(anyhow::anyhow!("Failed to receive file data: {}", e)),
    };

    let file_data_raw = match file_result {
        Ok(FileData::Json(data)) => data,
        Ok(_) => return Err(anyhow::anyhow!("Unexpected file format received")),
        Err(e) => return Err(anyhow::anyhow!("Failed to receive file data: {}", e)),
    };

    let config: Config = match serde_json::from_value(file_data_raw) {
        Ok(data) => data,
        Err(e) => return Err(anyhow::anyhow!("Failed to parse JSON data: {}", e)),
    };

    Ok(config)
}

pub async fn load_file(
    tx_filerequest: Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>,
    path: String,
    file_format: FileFormat,
) -> anyhow::Result<FileData> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let request = FileRequest::Read {
        path: path.into(),
        format: file_format,
        responder: tx,
    };

    let tx_filerequest = tx_filerequest.write().await;
    if let Err(e) = tx_filerequest.send(request).await {
        return Err(anyhow::anyhow!("Failed to send file read request: {}", e));
    }

    let file_result = match rx.await {
        Ok(result) => result,
        Err(e) => return Err(anyhow::anyhow!("Failed to receive file data: {}", e)),
    };

    let file_data_raw = match file_result {
        Ok(FileData::Json(data)) => FileData::Json(data),
        Ok(FileData::Toml(data)) => FileData::Toml(data),
        Ok(FileData::Text(data)) => FileData::Text(data),
        Ok(FileData::Png(data)) => FileData::Png(data),
        #[allow(unreachable_patterns)]
        Ok(_) => return Err(anyhow::anyhow!("Unexpected file format received")),
        Err(e) => return Err(anyhow::anyhow!("Failed to receive file data: {}", e)),
    };

    Ok(file_data_raw)
}

pub async fn check_file_exists(
    tx_filerequest: Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>,
    path: String
) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let request = FileRequest::Exists {
        path: path.into(),
        responder: tx
    };

    let tx_filerequest = tx_filerequest.write().await;
    if let Err(e) = tx_filerequest.send(request).await {
        tracing::error!("{}", e);
        return false;
    }

    let result = match rx.await {
        Ok(sth) => sth,
        Err(e) => {
            tracing::error!("{}", e);
            return false;
        }
    };

    match result {
        Ok(value) => value,
        Err(e) => {
            tracing::error!("{}", e);
            return false;
        }
    }
}

pub async fn write_file(
    tx_filerequest: Arc<RwLock<tokio::sync::mpsc::Sender<FileRequest>>>,
    path: String,
    file_data: &FileData,
) -> anyhow::Result<()> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    let (data, format) = match file_data {
        FileData::Json(value) => (FileData::Json(value.clone()), FileFormat::Json),
        FileData::Toml(value) => (FileData::Toml(value.clone()), FileFormat::Toml),
        FileData::Text(value) => (FileData::Text(value.clone()), FileFormat::Text),
        FileData::Png(value) => (FileData::Png(value.clone()), FileFormat::Png),
    };

    let request = FileRequest::Write {
        path: path.into(),
        format,
        data,
        responder: tx,
    };

    let tx_filerequest = tx_filerequest.write().await;
    if let Err(e) = tx_filerequest.send(request).await {
        tracing::error!("Failed to send file write request: {}", e);
        return Err(anyhow::anyhow!("Failed to send file write request: {}", e));
    }

    Ok(())
}
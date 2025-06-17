use tokio::sync::{
    mpsc,
    oneshot,
};
use crate::query::sat_query;

pub struct QueryRequest {
    pub query_value: String,
    pub responder: oneshot::Sender<Option<Vec<String>>>,
}

pub struct QueryHandler {
    json_file_path: String,
    toml_file_path: String,
    request_receiver: mpsc::Receiver<QueryRequest>,
}

impl QueryHandler {
    pub fn new(
        json_file_path: String,
        toml_file_path: String,
        request_receiver: mpsc::Receiver<QueryRequest>,
    ) -> Self {
        QueryHandler {
            json_file_path,
            toml_file_path,
            request_receiver,
        }
    }

    // run the query handler
    pub async fn run(mut self) {
        tracing::info!("Query handler started");
        
        while let Some(request) = self.request_receiver.recv().await {
            let result = sat_query::look_up_sat_status_from_json(
                &self.json_file_path,
                &self.toml_file_path,
                &request.query_value,
            );
            
            // send the result back to the requester
            let _ = request.responder.send(result);
        }
    }
}

#[derive(Clone)]
pub struct QueryClient {
    request_sender: mpsc::Sender<QueryRequest>,
}

impl QueryClient {
    pub fn new(request_sender: mpsc::Sender<QueryRequest>) -> Self {
        QueryClient { request_sender }
    }

    /// Sends a query request and waits for the response.
    pub async fn query(&self, query_value: String) -> Option<Vec<String>> {
        let (responder, receiver) = oneshot::channel();
        let cloned_query_value = query_value.clone();
        
        let request = QueryRequest {
            query_value,
            responder,
        };
        
        // send the request to the handler
        if let Err(e) = self.request_sender.send(request).await {
            tracing::error!("Failed to send query request: {}", e);
            return None;
        }
        
        // wait for the response
        match receiver.await {
            Ok(result) => {
                tracing::info!("Received query response for '{}'", cloned_query_value);
                result
            },
            Err(_) => {
                tracing::error!("Failed to receive query response");
                None
            }
        }
    }
}

pub fn init_query_system(
    json_file_path: String,
    toml_file_path: String,
) -> (QueryClient, QueryHandler) {
    let (request_sender, request_receiver) = mpsc::channel(32);
    
    let client = QueryClient::new(request_sender);
    let handler = QueryHandler::new(json_file_path, toml_file_path, request_receiver);
    
    (client, handler)
}
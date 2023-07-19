use axum::{extract::State, Json};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::{
    sampler::Sampler, FinishReason, GenerateRequest, OptionArray, ThreadRequest, TokenCounter,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CompletionRequest {
    pub prompt: OptionArray<String>,
    pub max_tokens: usize,
    pub stop: OptionArray<String>,
    pub temperature: f32,
    pub top_p: f32,
    pub presence_penalty: f32,
    pub frequency_penalty: f32,
}

impl Default for CompletionRequest {
    fn default() -> Self {
        Self {
            prompt: OptionArray::default(),
            max_tokens: 256,
            stop: OptionArray::default(),
            temperature: 1.0,
            top_p: 1.0,
            presence_penalty: 0.0,
            frequency_penalty: 0.0,
        }
    }
}

impl From<CompletionRequest> for GenerateRequest {
    fn from(value: CompletionRequest) -> Self {
        let CompletionRequest {
            prompt,
            max_tokens,
            stop,
            temperature,
            top_p,
            presence_penalty,
            frequency_penalty,
        } = value;

        let prompt = Vec::from(prompt).join("");
        let max_tokens = max_tokens.min(crate::MAX_TOKENS);
        let stop = stop.into();

        Self {
            prompt,
            max_tokens,
            stop,
            sampler: Sampler {
                temperature,
                top_p,
                presence_penalty,
                frequency_penalty,
            },
            occurrences: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletionChoice {
    pub text: String,
    pub index: usize,
    pub finish_reason: FinishReason,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletionResponse {
    pub object: String,
    pub choices: Vec<CompletionChoice>,
    #[serde(rename = "usage")]
    pub counter: TokenCounter,
}

pub async fn completions(
    State(sender): State<flume::Sender<ThreadRequest>>,
    Json(request): Json<CompletionRequest>,
) -> Json<CompletionResponse> {
    let (prompt_tokens_sender, prompt_tokens_receiver) = flume::unbounded();
    let (token_sender, token_receiver) = flume::unbounded();

    let _ = sender.send(ThreadRequest {
        request: crate::RequestKind::Completion(request),
        prompt_tokens_sender,
        token_sender,
    });

    let prompt_tokens = prompt_tokens_receiver
        .recv_async()
        .await
        .unwrap_or_default();
    let mut counter = TokenCounter {
        prompt_tokens,
        completion_tokens: 0,
        total_tokens: prompt_tokens,
    };

    let mut finish_reason = FinishReason::Null;
    let mut text = String::new();
    let mut stream = token_receiver.into_stream();

    while let Some(token) = stream.next().await {
        match token {
            crate::Token::Token(token) => {
                text += &token;
                counter.completion_tokens += 1;
                counter.total_tokens += 1;
            }
            crate::Token::EndOfText => {
                finish_reason = FinishReason::Stop;
                break;
            }
            crate::Token::CutOff => {
                finish_reason = FinishReason::Length;
                break;
            }
        }
    }

    Json(CompletionResponse {
        object: "text_completion".into(),
        choices: vec![CompletionChoice {
            text,
            index: 0,
            finish_reason,
        }],
        counter,
    })
}

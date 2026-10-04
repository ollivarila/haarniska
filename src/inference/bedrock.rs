//! Amazon Bedrock adapter: Claude through InvokeModel, on the AWS SDK.

mod messages;
mod refresh;

use aws_config::{BehaviorVersion, SdkConfig};
use aws_credential_types::provider::error::CredentialsError;
use aws_sdk_bedrockruntime::Client;
use aws_sdk_bedrockruntime::config::http::HttpResponse;
use aws_sdk_bedrockruntime::error::{DisplayErrorContext, ProvideErrorMetadata, SdkError};
use aws_sdk_bedrockruntime::operation::invoke_model_with_response_stream::{
    InvokeModelWithResponseStreamError, InvokeModelWithResponseStreamOutput,
};
use aws_sdk_bedrockruntime::primitives::Blob;
use aws_sdk_bedrockruntime::types::{PayloadPart, ResponseStream};
use futures_util::Stream;
use tokio::sync::Mutex;

use crate::inference::{Chunk, Error, Inference, Request};
use messages::Assembler;

/// Service error codes that mean the credentials are expired or unknown.
const AUTH_ERROR_CODES: [&str; 3] = [
    "ExpiredTokenException",
    "ExpiredToken",
    "UnrecognizedClientException",
];

pub struct BedrockInference {
    model: String,
    config: ConfigSource,
    auth_refresh: Option<String>,
    client: Mutex<Option<Client>>,
}

impl BedrockInference {
    /// `model` is a model ID, or an inference profile ID or ARN.
    ///
    /// AWS configuration is loaded on the first call, the standard way: the
    /// profile from `AWS_PROFILE`, the region from `AWS_REGION`.
    pub fn new(model: &str) -> Self {
        Self::with_source(model, ConfigSource::Load(None))
    }

    /// Uses `config` as is, instead of loading it from the environment.
    pub fn from_config(model: &str, config: SdkConfig) -> Self {
        Self::with_source(model, ConfigSource::Given(Box::new(config)))
    }

    /// Uses the named AWS profile instead of `AWS_PROFILE`. Has no effect
    /// after [`from_config`](Self::from_config).
    pub fn with_profile(mut self, name: &str) -> Self {
        if let ConfigSource::Load(profile) = &mut self.config {
            *profile = Some(name.to_owned());
        }
        self
    }

    /// A shell command that gets new credentials, e.g.
    /// `aws sso login --profile dev`. When a call fails because the
    /// credentials are missing or expired, it is run once and the call is
    /// retried.
    pub fn with_auth_refresh(mut self, command: &str) -> Self {
        self.auth_refresh = Some(command.to_owned());
        self
    }

    fn with_source(model: &str, config: ConfigSource) -> Self {
        Self {
            model: model.to_owned(),
            config,
            auth_refresh: None,
            client: Mutex::new(None),
        }
    }
}

impl Inference for BedrockInference {
    fn infer(&self, request: Request<'_>) -> impl Stream<Item = Result<Chunk, Error>> {
        let body = messages::to_body(request).to_string();

        async_stream::stream! {
            let mut output = match self.send(body).await {
                Ok(output) => output,
                Err(error) => {
                    yield Err(error);
                    return;
                }
            };
            let mut assembler = Assembler::default();
            loop {
                let event = match output.body.recv().await {
                    Ok(Some(ResponseStream::Chunk(PayloadPart { bytes: Some(bytes), .. }))) => bytes,
                    Ok(Some(_)) => continue,
                    Ok(None) => {
                        yield Err(Error::new("the stream ended before the reply"));
                        return;
                    }
                    Err(error) => {
                        yield Err(to_error(error));
                        return;
                    }
                };
                let Some(item) = assembler.push(event.as_ref()) else {
                    continue;
                };
                let is_last = !matches!(item, Ok(Chunk::Text(_)));
                yield item;
                if is_last {
                    return;
                }
            }
        }
    }
}

/// Where the AWS configuration comes from.
enum ConfigSource {
    /// Loaded from the environment, and again after an auth refresh, so new
    /// credentials are picked up.
    /// With a profile name, if not the default.
    Load(Option<String>),
    Given(Box<SdkConfig>),
}

type SendError = SdkError<InvokeModelWithResponseStreamError, HttpResponse>;

impl BedrockInference {
    async fn send(&self, body: String) -> Result<InvokeModelWithResponseStreamOutput, Error> {
        let result = self.invoke(&body).await;
        match (&self.auth_refresh, result) {
            (Some(command), Err(error)) if is_auth_failure(&error) => {
                refresh::run(command).await?;
                *self.client.lock().await = None;
                self.invoke(&body).await.map_err(to_error)
            }
            (_, result) => result.map_err(to_error),
        }
    }

    async fn invoke(&self, body: &str) -> Result<InvokeModelWithResponseStreamOutput, SendError> {
        self.client()
            .await
            .invoke_model_with_response_stream()
            .model_id(&self.model)
            .content_type("application/json")
            .accept("application/json")
            .body(Blob::new(body))
            .send()
            .await
    }

    async fn client(&self) -> Client {
        let mut client = self.client.lock().await;
        if let Some(client) = client.as_ref() {
            return client.clone();
        }
        let config = match &self.config {
            ConfigSource::Load(profile) => {
                let mut loader = aws_config::defaults(BehaviorVersion::latest());
                if let Some(profile) = profile {
                    loader = loader.profile_name(profile);
                }
                &loader.load().await
            }
            ConfigSource::Given(config) => config,
        };
        client.insert(Client::new(config)).clone()
    }
}

fn is_auth_failure(error: &SendError) -> bool {
    let is_auth_code = error
        .code()
        .is_some_and(|code| AUTH_ERROR_CODES.contains(&code));
    is_auth_code
        || std::iter::successors(Some(error as &(dyn std::error::Error + 'static)), |error| {
            error.source()
        })
        .any(|error| error.is::<CredentialsError>())
}

fn to_error(error: impl std::error::Error) -> Error {
    Error::new(DisplayErrorContext(error).to_string())
}

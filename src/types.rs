use crate::ProviderName;

#[derive(Clone, Debug)]
pub struct Provider {
    pub name: ProviderName,
    pub api_key: String,
    pub model: Option<String>,
    pub base_url: Option<String>,
}

impl Provider {
    pub fn new(name: ProviderName, api_key: impl Into<String>) -> Self {
        Self {
            name,
            api_key: api_key.into(),
            model: None,
            base_url: None,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct Request {
    pub system: Option<String>,
    pub user: Option<String>,
    pub messages: Vec<Message>,
    pub schema: Option<String>,
    pub files: Vec<File>,
    pub images: Vec<Image>,
}

impl Request {
    pub fn new(user: impl Into<String>) -> Self {
        Self {
            system: None,
            user: Some(user.into()),
            messages: Vec::new(),
            schema: None,
            files: Vec::new(),
            images: Vec::new(),
        }
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    pub fn with_messages(mut self, messages: Vec<Message>) -> Self {
        self.messages = messages;
        self
    }

    pub fn with_schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    pub fn with_files(mut self, files: Vec<File>) -> Self {
        self.files = files;
        self
    }

    pub fn with_images(mut self, images: Vec<Image>) -> Self {
        self.images = images;
        self
    }
}

#[derive(Clone, Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct File {
    pub id: String,
    pub uri: String,
    pub mime_type: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Image {
    pub url: String,
    pub mime_type: String,
    pub detail: String,
}

#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,
    pub run: std::sync::Arc<
        dyn Fn(serde_json::Map<String, serde_json::Value>) -> Result<String, String> + Send + Sync,
    >,
}

impl Tool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        schema: serde_json::Value,
        run: impl Fn(serde_json::Map<String, serde_json::Value>) -> Result<String, String>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            schema,
            run: std::sync::Arc::new(run),
        }
    }

    pub fn run(&self, args: serde_json::Map<String, serde_json::Value>) -> Result<String, String> {
        (self.run)(args)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    pub input: u32,
    pub output: u32,
    pub cache_write: u32,
    pub cache_read: u32,
    pub reasoning: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Response {
    pub text: String,
    pub usage: Usage,
}

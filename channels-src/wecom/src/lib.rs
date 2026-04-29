wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

include!("../../shared_webhook_channel/src/impl.rs");

export!(GenericWebhookChannel);

// use tracing_subscriber::{prelude::*, util::SubscriberInitExt, EnvFilter};
use opentelemetry::{global, sdk::propagation::TraceContextPropagator};
use opentelemetry_gcloud_trace::GcpCloudTraceExporterBuilder;
use tracing::{info, instrument};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Registry;

pub fn tracing() {
    // Set up the GCP Cloud Trace exporter
    let exporter = GcpCloudTraceExporterBuilder::for_default_project_id()
        .await?
        .install()
        .await?;

    // Create a new OpenTelemetry tracer and provider
    let tracer = opentelemetry_sdk::trace::Tracer::new(opentelemetry_sdk::trace::Config {
        resource: Some(opentelemetry_sdk::Resource::new(vec![
            opentelemetry_semantic_conventions::resource::SERVICE_NAME.string("proof.zk.type1.zero-bin.leader"),
        ])),
        ..Default::default()
    });
    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_simple_exporter(exporter)
        .with_config(opentelemetry_sdk::trace::Config {
            default_sampler: Box::new(opentelemetry_sdk::trace::Sampler::AlwaysOn),
            ..Default::default()
        })
        .build();
    
    // Set up the OpenTelemetry layer for tracing
    let opentelemetry_layer = OpenTelemetryLayer::new(tracer);

    // Create a subscriber and add the OpenTelemetry layer
    let subscriber = Registry::default().with(opentelemetry_layer);

    // Set the global subscriber
    tracing::subscriber::set_global_default(subscriber)?;
    
    // Set the global propagator
    global::set_text_map_propagator(TraceContextPropagator::new());
}

// pub fn tracing() {
//     tracing_subscriber::Registry::default()
//         .with(
//             tracing_subscriber::fmt::layer()
//                 .with_ansi(false)
//                 .compact()
//                 .with_filter(EnvFilter::from_default_env()),
//         )
//         .init();
// }

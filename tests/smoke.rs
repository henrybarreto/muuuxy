use testcontainers::{GenericImage, core::WaitFor, runners::AsyncRunner};

#[tokio::test]
async fn smoke_test() {
    // NOTE: As testcontainers on Rust doesn't support building the image, we should run the build
    // manually with `docker build -t muuuxy:test .` to continue the smoke test.
    const CONTAINER_NAME: &str = "muuuxy";
    const CONTAINER_VERSION: &str = "test";

    let _container = GenericImage::new(CONTAINER_NAME, CONTAINER_VERSION)
        .with_wait_for(WaitFor::healthcheck())
        .start()
        .await
        .unwrap()
        .rm()
        .await
        .unwrap();
}

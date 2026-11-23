//! Live check against the local Docker daemon. Ignored by default:
//! `cargo test -p docker -- --ignored --nocapture`.

#[test]
#[ignore = "needs a running local Docker daemon"]
fn talks_to_the_local_daemon() {
    let home = std::env::var("HOME").unwrap();
    let socket = docker::resolve_socket(
        std::env::var("DOCKER_HOST").ok().as_deref(),
        std::path::Path::new(&home),
    )
    .expect("no Docker socket found");
    let docker = docker::Docker::connect(&socket).expect("daemon answers /_ping");
    let containers = docker.containers().expect("list containers");
    println!("socket {} — {} containers", socket.display(), containers.len());
    for c in containers.iter().take(5) {
        println!(
            "  {} {} {} {:?}",
            c.short_id(),
            c.state.label(),
            c.image,
            c.public_ports()
        );
    }
}

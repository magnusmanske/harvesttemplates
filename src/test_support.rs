//! Shared test helpers. DB tests need Docker (as on GitHub's runners).

use crate::storage::Store;
use testcontainers_modules::mariadb::Mariadb;
use testcontainers_modules::testcontainers::ContainerAsync;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

/// A migrated store in a fresh MariaDB container. Keep the container alive for the test.
pub async fn test_store() -> (ContainerAsync<Mariadb>, Store) {
    let container = Mariadb::default()
        .start()
        .await
        .expect("Docker must be running for DB tests");
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let store = Store::new(&format!("mysql://root@127.0.0.1:{port}/test"), 4).unwrap();
    store.migrate().await.unwrap();
    (container, store)
}

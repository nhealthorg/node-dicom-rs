use s3::creds::Credentials;
use s3::{Bucket, Region};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

/// S3 storage configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[napi(object)]
pub struct S3Config {
  /// S3 bucket name
  pub bucket: String,
  /// AWS access key ID
  pub access_key: String,
  /// AWS secret access key
  pub secret_key: String,
  /// S3 endpoint (e.g., "http://localhost:9000" for MinIO)
  pub endpoint: Option<String>,
}

/// Build an S3 bucket instance from configuration
pub fn build_s3_bucket(config: &S3Config) -> Bucket {
  let endpoint = config
    .endpoint
    .clone()
    .unwrap_or_else(|| "http://localhost:7070".to_string());
  let region = Region::Custom {
    region: "us-east-1".to_owned(),
    endpoint,
  };
  let credentials = Credentials::new(
    Some(&config.access_key),
    Some(&config.secret_key),
    None,
    None,
    None,
  )
  .expect("Invalid S3 credentials");

  *Bucket::new(&config.bucket, region, credentials)
    .expect("Failed to create S3 bucket")
    .with_path_style()
}

/// Check S3 connectivity
pub async fn check_s3_connectivity(bucket: &Bucket) {
  match bucket.exists().await {
    Ok(_) => {
      info!(
        "S3 connectivity check succeeded for bucket: {}",
        bucket.name()
      );
    }
    Err(e) => {
      error!(
        "S3 connectivity check failed for bucket: {}: {}",
        bucket.name(),
        e
      );
    }
  }
}

/// Get object from S3
pub async fn s3_get_object(
  bucket: &Bucket,
  path: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
  let response = match bucket.get_object(path).await {
    Ok(r) => r,
    Err(e) => return Err(Box::new(e)),
  };
  let code = response.status_code();
  if code == 200 {
    let bytes = response.bytes().to_vec();
    info!("Downloaded S3 object '{}': {} bytes", path, bytes.len());
    Ok(bytes)
  } else {
    Err(format!("S3 get_object error: HTTP {}", code).into())
  }
}

/// Get an object from S3, optionally pinning the request to an exact version.
/// Unlike `s3_get_object`, this helper does not log the object key.
pub async fn s3_get_object_at_version(
  bucket: &Bucket,
  path: &str,
  version_id: Option<&str>,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
  let mut request_bucket = bucket.clone();
  if let Some(version_id) = version_id {
    request_bucket.add_query("versionId", version_id);
  }

  let response = request_bucket.get_object(path).await?;
  let status = response.status_code();
  if status == 200 {
    Ok(response.bytes().to_vec())
  } else {
    Err(format!("S3 get object failed with HTTP {}", status).into())
  }
}

/// Put object to S3
pub async fn s3_put_object(
  bucket: &Bucket,
  path: &str,
  data: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let response = match bucket.put_object(path, data).await {
    Ok(r) => r,
    Err(e) => return Err(Box::new(e)),
  };
  let code = response.status_code();
  if code == 200 || code == 201 {
    Ok(())
  } else {
    Err(format!("S3 put_object error: HTTP {}", code).into())
  }
}

/// List objects in S3 with given prefix
pub async fn s3_list_objects(
  bucket: &Bucket,
  prefix: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
  let results = match bucket.list(prefix.to_string(), None).await {
    Ok(r) => r,
    Err(e) => return Err(Box::new(e)),
  };

  let mut objects = Vec::new();
  for list in results {
    for obj in list.contents {
      objects.push(obj.key);
    }
  }

  Ok(objects)
}

#[cfg(test)]
mod tests {
  use super::*;
  use tokio::io::{AsyncReadExt, AsyncWriteExt};
  use tokio::net::TcpListener;

  #[tokio::test]
  async fn get_object_at_version_pins_each_request_and_does_not_retry_latest() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
      let responses = [
        ("v1", "older bytes", 200),
        ("v2", "newer bytes", 200),
        ("missing", "", 404),
      ];

      for (version_id, body, status) in responses {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
          let mut chunk = [0; 1024];
          let size = socket.read(&mut chunk).await.unwrap();
          if size == 0 {
            break;
          }
          request.extend_from_slice(&chunk[..size]);
          if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
          }
        }

        let request = String::from_utf8_lossy(&request);
        let request_line = request.lines().next().unwrap_or_default();
        assert!(request_line.contains(&format!("versionId={version_id}")));

        let reason = if status == 200 { "OK" } else { "Not Found" };
        let response = format!(
          "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
          body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
      }
    });

    let bucket = build_s3_bucket(&S3Config {
      bucket: "test-bucket".to_string(),
      access_key: "test-access".to_string(),
      secret_key: "test-secret".to_string(),
      endpoint: Some(endpoint),
    });

    assert_eq!(
      s3_get_object_at_version(&bucket, "opaque-key", Some("v1"))
        .await
        .unwrap(),
      b"older bytes"
    );
    assert_eq!(
      s3_get_object_at_version(&bucket, "opaque-key", Some("v2"))
        .await
        .unwrap(),
      b"newer bytes"
    );
    assert!(
      s3_get_object_at_version(&bucket, "opaque-key", Some("missing"))
        .await
        .is_err()
    );

    server.await.unwrap();
  }
}

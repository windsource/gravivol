use std::collections::{HashMap, HashSet};

use base64::{prelude::BASE64_STANDARD, Engine};
use json_patch::diff;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    namespace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    labels: Option<HashMap<String, String>>,
}

#[derive(Debug)]
struct Label {
    key: String,
    value: String,
}

impl Label {
    pub fn from_pvc(pvc: &Pvc) -> Label {
        Label {
            key: format!("{}.gravivol.fonona.net/{}", pvc.namespace, pvc.claim_name),
            value: "true".to_string(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistentVolumeClaim {
    claim_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Volume {
    #[serde(skip_serializing_if = "Option::is_none")]
    persistent_volume_claim: Option<PersistentVolumeClaim>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Spec {
    #[serde(skip_serializing_if = "Option::is_none")]
    volumes: Option<Vec<Volume>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    affinity: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Pod {
    kind: String,
    api_version: String,
    metadata: Metadata,
    spec: Spec,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    uid: String,
    object: Pod,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    uid: String,
    allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    patch_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    patch: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionReview {
    pub api_version: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<Request>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response: Option<Response>,
}

fn create_patch(pod: &Pod, pvcs: Vec<String>) -> String {
    let labels = pvcs
        .iter()
        .map(|p| {
            Label::from_pvc(&Pvc {
                namespace: pod.metadata.namespace.to_owned(),
                claim_name: p.to_owned(),
            })
        })
        .collect::<Vec<Label>>();

    let mut new_pod = pod.to_owned();

    // Add labels to metadata
    if new_pod.metadata.labels.is_none() {
        new_pod.metadata.labels = Some(HashMap::new())
    }
    if let Some(new_labels) = &mut new_pod.metadata.labels {
        for label in &labels {
            new_labels.insert(label.key.to_owned(), label.value.to_owned());
        }
    }

    // Add affinity
    if new_pod.spec.affinity.is_none() {
        new_pod.spec.affinity = Some(Value::Null);
    }
    if let Some(affinity) = &mut new_pod.spec.affinity {
        if affinity["podAffinity"]["requiredDuringSchedulingIgnoredDuringExecution"].is_null() {
            affinity["podAffinity"]["requiredDuringSchedulingIgnoredDuringExecution"] = json!([]);
        }
        if let Value::Array(the_array) =
            &mut affinity["podAffinity"]["requiredDuringSchedulingIgnoredDuringExecution"]
        {
            let mut entry = json!({
                "labelSelector": {
                    "matchLabels": {
                    }
                },
                "topologyKey": "kubernetes.io/hostname",
            });
            for label in &labels {
                entry["labelSelector"]["matchLabels"][&label.key] =
                    Value::String(label.value.to_owned());
            }
            the_array.push(entry);
        }
    }

    let original_pod = serde_json::to_value(pod).expect("Cannot serialize pod");
    let patched_pod = serde_json::to_value(new_pod).expect("Cannot serialize new pod");

    let result_patch =
        serde_json::to_string(&diff(&original_pod, &patched_pod)).expect("Cannot serialize patch");

    log::info!("Patch: {result_patch}");
    result_patch
}

#[derive(Eq, Hash, PartialEq)]
struct Pvc {
    namespace: String,
    claim_name: String,
}

impl Pvc {
    fn from_config_entry(entry: &str) -> Option<Pvc> {
        let entry_parts: Vec<&str> = entry.split('/').collect();
        if entry_parts.len() == 2 {
            Some(Pvc {
                namespace: entry_parts[0].to_owned(),
                claim_name: entry_parts[1].to_owned(),
            })
        } else {
            log::error!(
                "Config entry is not in the format <namespace>/<claim name> : {}",
                entry
            );
            None
        }
    }
}

pub struct Controller {
    // If set is empty, all PVCs will be handled
    pvcs_to_handle: HashSet<Pvc>,
}

impl Controller {
    /// config is comma separated string with PVCs to consider
    pub fn new(config: &str) -> Controller {
        let mut pvcs = HashSet::new();
        for config_entry in config.split(',') {
            if !config_entry.is_empty() {
                if let Some(pvc) = Pvc::from_config_entry(config_entry) {
                    pvcs.insert(pvc);
                }
            }
        }
        Controller {
            pvcs_to_handle: pvcs,
        }
    }

    fn pvc_needs_handling(&self, namespace: &str, claim_name: &str) -> bool {
        let pvc = Pvc {
            namespace: namespace.to_owned(),
            claim_name: claim_name.to_owned(),
        };
        if self.pvcs_to_handle.is_empty() {
            true
        } else {
            self.pvcs_to_handle.contains(&pvc)
        }
    }

    pub fn mutate(
        &self,
        review: AdmissionReview,
    ) -> Result<AdmissionReview, Box<dyn std::error::Error>> {
        if let Some(request) = review.request {
            let mut pvcs_found: Vec<String> = Vec::new();
            let mut review = AdmissionReview {
                api_version: review.api_version.clone(),
                kind: review.kind.clone(),
                request: None,
                response: Some(Response {
                    uid: request.uid.clone(),
                    allowed: true,
                    patch_type: None,
                    patch: None,
                }),
            };

            if &request.object.kind != "Pod" {
                log::error!("Object is not a Pod but {}", &request.object.kind);
                return Ok(review);
            }

            // Extract PVCs
            if let Some(volumes) = &request.object.spec.volumes {
                for vol in volumes {
                    if let Some(pvc) = &vol.persistent_volume_claim {
                        if self
                            .pvc_needs_handling(&request.object.metadata.namespace, &pvc.claim_name)
                        {
                            log::info!("Pod uses matching PVC {}", pvc.claim_name);
                            pvcs_found.push(pvc.claim_name.to_owned());
                        }
                    }
                }
            }

            if !pvcs_found.is_empty() {
                let patch = create_patch(&request.object, pvcs_found);

                let mut response = review.response.unwrap();
                response.patch_type = Some("JSONPatch".to_owned());
                response.patch = Some(BASE64_STANDARD.encode(patch.as_bytes()));
                review.response = Some(response);
            }

            Ok(review)
        } else {
            Err("No request in AdmissionReview found!".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use json_patch::{patch, Patch};
    use serde_json::json;

    use super::*;

    #[test]
    fn test_pod_no_volumes() {
        let data = json!(
        {
            "apiVersion": "admission.k8s.io/v1",
            "kind": "AdmissionReview",
            "request": {
                "uid": "26973DA1-B488-4F59-B062-461C6BDCAD83",
                "object": {
                    "kind": "Pod",
                    "apiVersion": "v1",
                    "metadata": {
                        "generateName": "bla-6b47d48686-",
                        "namespace": "default",
                        "creationTimestamp": null,
                        "labels": {
                            "app.kubernetes.io/instance": "bla",
                            "app.kubernetes.io/managed-by": "Helm",
                            "app.kubernetes.io/name": "bla",
                            "app.kubernetes.io/version": "1.16.0"
                        }
                    },
                    "spec": {}
                }
            }
        });
        let review: AdmissionReview = serde_json::from_value(data).expect("Failed to parse JSON");
        let controller = Controller::new("");
        let response = controller.mutate(review).unwrap();

        assert_eq!(response.api_version, "admission.k8s.io/v1");
        assert_eq!(response.kind, "AdmissionReview");
        match response.response {
            Some(res) => {
                assert_eq!(res.uid, "26973DA1-B488-4F59-B062-461C6BDCAD83");
                assert!(res.allowed);
                assert_eq!(res.patch, None);
                assert_eq!(res.patch_type, None);
            }
            None => panic!("Expected Some(response)"),
        }
    }

    #[test]
    fn test_pod_with_matching_and_non_matching_pvc() {
        let config = "default/myvol1,foo/myvol2";

        let mut pod = json!({
            "kind": "Pod",
            "apiVersion": "v1",
            "metadata": {
                "generateName": "bla-6b47d48686-",
                "namespace": "default",
                "creationTimestamp": null,
                "labels": {
                    "app.kubernetes.io/instance": "bla",
                    "app.kubernetes.io/managed-by": "Helm",
                    "app.kubernetes.io/name": "bla",
                    "app.kubernetes.io/version": "1.16.0"
                }
            },
            "spec": {
                "volumes": [
                    {
                        "name": "somename",
                        "persistentVolumeClaim": {
                            "claimName": "myvol1"
                        }
                    },
                    {
                        "name": "othername",
                        "persistentVolumeClaim": {
                            "claimName": "myvol2"
                        }
                    }
                ]
            }
        });

        let expected_patched_pod = json!({
            "kind": "Pod",
            "apiVersion": "v1",
            "metadata": {
                "generateName": "bla-6b47d48686-",
                "namespace": "default",
                "creationTimestamp": null,
                "labels": {
                    "app.kubernetes.io/instance": "bla",
                    "app.kubernetes.io/managed-by": "Helm",
                    "app.kubernetes.io/name": "bla",
                    "app.kubernetes.io/version": "1.16.0",
                     "default.gravivol.fonona.net/myvol1": "true",
                }
            },
            "spec": {
                "volumes": [
                    {
                        "name": "somename",
                        "persistentVolumeClaim": {
                            "claimName": "myvol1"
                        }
                    },
                    {
                        "name": "othername",
                        "persistentVolumeClaim": {
                            "claimName": "myvol2"
                        }
                    }
                ],
                "affinity": {
                    "podAffinity": {
                        "requiredDuringSchedulingIgnoredDuringExecution": [
                        {
                            "labelSelector": {
                                "matchLabels": {
                                    "default.gravivol.fonona.net/myvol1": "true",
                                }
                            },
                            "topologyKey": "kubernetes.io/hostname",
                        }]
                    }
                }
            }
        });

        let data = json!({
            "apiVersion": "admission.k8s.io/v1",
            "kind": "AdmissionReview",
            "request": {
                "uid": "26973DA1-B488-4F59-B062-461C6BDCAD83",
                "object": pod.clone(),
            }
        });
        let review: AdmissionReview = serde_json::from_value(data).expect("Failed to parse JSON");
        let controller = Controller::new(config);
        let response = controller.mutate(review).unwrap();

        assert_eq!(response.api_version, "admission.k8s.io/v1");
        assert_eq!(response.kind, "AdmissionReview");
        match response.response {
            Some(res) => {
                assert_eq!(res.uid, "26973DA1-B488-4F59-B062-461C6BDCAD83");
                assert_eq!(res.patch_type, Some("JSONPatch".to_owned()));
                let patch_string = String::from_utf8(
                    BASE64_STANDARD
                        .decode(res.patch.expect("No patch in response"))
                        .expect("Cannot decode base64"),
                )
                .expect("Invalid UTF-8");
                let patch_json: Patch =
                    serde_json::from_str(&patch_string).expect("Cannot parse JSON patch");
                patch(&mut pod, &patch_json).expect("Patch failed");
                assert_eq!(pod, expected_patched_pod);
            }
            None => panic!("Expected Some(response)"),
        }
    }

    #[test]
    fn test_empty_config() {
        let config = "";

        let mut pod = json!({
            "kind": "Pod",
            "apiVersion": "v1",
            "metadata": {
                "generateName": "bla-6b47d48686-",
                "namespace": "default",
                "creationTimestamp": null,
                "labels": {
                    "app.kubernetes.io/instance": "bla",
                    "app.kubernetes.io/managed-by": "Helm",
                    "app.kubernetes.io/name": "bla",
                    "app.kubernetes.io/version": "1.16.0"
                }
            },
            "spec": {
                "volumes": [
                    {
                        "name": "somename",
                        "persistentVolumeClaim": {
                            "claimName": "myvol1"
                        }
                    }
                ]
            }
        });

        let expected_patched_pod = json!({
            "kind": "Pod",
            "apiVersion": "v1",
            "metadata": {
                "generateName": "bla-6b47d48686-",
                "namespace": "default",
                "creationTimestamp": null,
                "labels": {
                    "app.kubernetes.io/instance": "bla",
                    "app.kubernetes.io/managed-by": "Helm",
                    "app.kubernetes.io/name": "bla",
                    "app.kubernetes.io/version": "1.16.0",
                    "default.gravivol.fonona.net/myvol1": "true",
                }
            },
            "spec": {
                "volumes": [
                    {
                        "name": "somename",
                        "persistentVolumeClaim": {
                            "claimName": "myvol1"
                        }
                    }
                ],
                "affinity": {
                    "podAffinity": {
                        "requiredDuringSchedulingIgnoredDuringExecution": [
                        {
                            "labelSelector": {
                                "matchLabels": {
                                    "default.gravivol.fonona.net/myvol1": "true",
                                }
                            },
                            "topologyKey": "kubernetes.io/hostname",
                        }]
                    }
                }
            }
        });

        let data = json!({
            "apiVersion": "admission.k8s.io/v1",
            "kind": "AdmissionReview",
            "request": {
                "uid": "26973DA1-B488-4F59-B062-461C6BDCAD83",
                "object": pod.clone(),
            }
        });
        let review: AdmissionReview = serde_json::from_value(data).expect("Failed to parse JSON");
        let controller = Controller::new(config);
        let response = controller.mutate(review).unwrap();

        assert_eq!(response.api_version, "admission.k8s.io/v1");
        assert_eq!(response.kind, "AdmissionReview");
        match response.response {
            Some(res) => {
                assert_eq!(res.uid, "26973DA1-B488-4F59-B062-461C6BDCAD83");
                assert_eq!(res.patch_type, Some("JSONPatch".to_owned()));
                let patch_string = String::from_utf8(
                    BASE64_STANDARD
                        .decode(res.patch.expect("No patch in response"))
                        .expect("Cannot decode base64"),
                )
                .expect("Invalid UTF-8");
                let patch_json: Patch =
                    serde_json::from_str(&patch_string).expect("Cannot parse JSON patch");
                patch(&mut pod, &patch_json).expect("Patch failed");
                assert_eq!(pod, expected_patched_pod);
            }
            None => panic!("Expected Some(response)"),
        }
    }

    #[test]
    fn test_create_patch_pure_pod() {
        let pod_before = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "namespace": "foo"
            },
            "spec": {
                "containers": [
                    {
                        "name": "my-container",
                        "image": "nginx"
                    }
                ]
            }
        });

        let pvcs = vec!["myvol1".to_owned(), "myvol2".to_owned()];

        let mut pod_after = pod_before.to_owned();

        pod_after["metadata"]["labels"] = json!({
            "foo.gravivol.fonona.net/myvol1": "true",
            "foo.gravivol.fonona.net/myvol2": "true",
        });

        pod_after["spec"]["affinity"] = json!({
            "podAffinity": {
                "requiredDuringSchedulingIgnoredDuringExecution": [
                {
                    "labelSelector": {
                        "matchLabels": {
                            "foo.gravivol.fonona.net/myvol1": "true",
                            "foo.gravivol.fonona.net/myvol2": "true",
                        }
                    },
                    "topologyKey": "kubernetes.io/hostname",
                }]
            }
        });

        let pod: Pod = serde_json::from_value(pod_before.to_owned()).unwrap();
        let created_patch: Patch = serde_json::from_str(&create_patch(&pod, pvcs)).unwrap();

        let mut pod_patched = pod_before.to_owned();
        patch(&mut pod_patched, &created_patch).unwrap();

        assert_eq!(pod_patched, pod_after);
    }

    #[test]
    fn test_create_patch_existing_labels_and_affinity() {
        let pod_before = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "namespace": "my-namespace",
                "labels": {
                    "mylabel1": "myvalue1"
                }
            },
            "spec": {
                "containers": [
                    {
                        "name": "my-container",
                        "image": "nginx"
                    }
                ],
                "affinity": {
                    "podAffinity": {
                        "requiredDuringSchedulingIgnoredDuringExecution": [
                        {
                            "labelSelector": {
                                "matchLabels": {
                                    "somelabel": "somevalue",
                                }
                            },
                            "topologyKey": "somekey",
                        }]
                    }
                }
            }
        });

        let pvcs = vec!["myvol1".to_owned(), "myvol2".to_owned()];

        let mut pod_after = pod_before.to_owned();

        pod_after["metadata"]["labels"]["my-namespace.gravivol.fonona.net/myvol1"] =
            Value::String("true".to_owned());
        pod_after["metadata"]["labels"]["my-namespace.gravivol.fonona.net/myvol2"] =
            Value::String("true".to_owned());

        if let Value::Array(the_array) = &mut pod_after["spec"]["affinity"]["podAffinity"]
            ["requiredDuringSchedulingIgnoredDuringExecution"]
        {
            the_array.push(json!({
                "labelSelector": {
                    "matchLabels": {
                        "my-namespace.gravivol.fonona.net/myvol1": "true",
                        "my-namespace.gravivol.fonona.net/myvol2": "true",
                    }
                },
                "topologyKey": "kubernetes.io/hostname",
            }));
        }

        let pod: Pod = serde_json::from_value(pod_before.to_owned()).unwrap();
        let created_patch: Patch = serde_json::from_str(&create_patch(&pod, pvcs)).unwrap();

        let mut pod_patched = pod_before.to_owned();
        patch(&mut pod_patched, &created_patch).unwrap();

        assert_eq!(pod_patched, pod_after);
    }
}

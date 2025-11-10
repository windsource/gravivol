# Gravivol

Gravivol is a Kubernetes [mutating admission webhook](https://kubernetes.io/docs/reference/access-authn-authz/admission-controllers/#mutatingadmissionwebhook). It a is workaround for a problem in Kubernetes, that pods using the same PVC with ReadWriteOnce get scheduled on different nodes and thus cannot run (see the open issue 
in Kubernetes [#103305](https://github.com/kubernetes/kubernetes/issues/103305) and also [hetznercloud/csi-driver #319](
https://github.com/hetznercloud/csi-driver/issues/319)).

For every pod, Gravivol checks the usage of configured PVCs.
If detected, Gravivol adds labels and pod affinities such that pods using the same PVCs get scheduled on the same node.

## Example

For a pod in the default namespace using the PVCs `data-vol` and `db-vol`, Gravivol will add

```yaml
metadata:
  labels:
    "default.gravivol.fonona.net/data-vol": "true"
    "default.gravivol.fonona.net/db-vol": "true"
spec:
  affinity:
    podAffinity:
      requiredDuringSchedulingIgnoredDuringExecution:
      - labelSelector:
          matchLabels:
            "default.gravivol.fonona.net/data-vol": "true"
            "default.gravivol.fonona.net/db-vol": "true"
        topologyKey: kubernetes.io/hostname
```    

## Installation

Before installing Gravivol, make sure that [cert-manager](https://cert-manager.io) has been installed:

```shell
helm install \
  cert-manager oci://quay.io/jetstack/charts/cert-manager \
  --version v1.19.1 \
  --namespace cert-manager \
  --create-namespace \
  --set crds.enabled=true
```

To install Gravivol, a Helm chart is provided that contains all required resources. This can be installed with:

```shell
helm repo add gravivol https://windsource.github.io/gravivol
helm install gravivol gravivol/gravivol
```

Configuration:

| Value | Description | Default |
| ----- | ----------- | ------- |
| pvcConfig | The list of PVCs to be handled. Format is a comma separated list of `<namespace>/<PVC>`. If the list is empty, all PVCs in all namespace will be handled. | "" |

## Reference

For the concept of admission webhooks see the Kubernetes page on [Dynamic Admission Control](https://kubernetes.io/docs/reference/access-authn-authz/extensible-admission-controllers/).
Webhooks are sent as POST requests, with Content-Type: `application/json`, with an `AdmissionReview` API object serialized to JSON as the body (see [Request](https://kubernetes.io/docs/reference/access-authn-authz/extensible-admission-controllers/#request) for an example and also the [reference](https://kubernetes.io/docs/reference/config-api/apiserver-admission.v1/#admission-k8s-io-v1-AdmissionReview)).

Example:

```json
{
  "apiVersion": "admission.k8s.io/v1",
  "kind": "AdmissionReview",
  "request": {
    # Random uid uniquely identifying this admission call
    "uid": "705ab4f5-6393-11e8-b7cc-42010a800002",
  ...
}
```

Webhooks [responds](https://kubernetes.io/docs/reference/access-authn-authz/extensible-admission-controllers/#response) with a 200 HTTP status code as body containing an AdmissionReview object (in the same version they were sent).

Example:

```json
{
  "apiVersion": "admission.k8s.io/v1",
  "kind": "AdmissionReview",
  "response": {
    "uid": "<value from request.uid>",
    "allowed": true
  }
}
```

When allowing a request, a mutating admission webhook may optionally modify the incoming object as well. This is done using the `patch` and `patchType` fields in the response.


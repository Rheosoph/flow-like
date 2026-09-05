use super::{Config, Slot, now};
use serde_json::{Map, Value, json};

pub const INSTALLATION: &str = "app.kubernetes.io/instance";
pub const COMPONENT: &str = "app.kubernetes.io/component";
pub const SLOT: &str = "flow-like.io/slot";
pub const RUN: &str = "flow-like.io/run";
pub const MANAGER: &str = "flow-like.io/manager";
pub const DEADLINE: &str = "flow-like.io/deadline";

impl Config {
    pub(super) fn labels(&self, slot: Option<&Slot>, component: Option<&str>) -> Value {
        let mut labels = json!({"app.kubernetes.io/name":self.app_name, INSTALLATION:self.common.installation, MANAGER:self.pod_uid});
        if let Some(slot) = slot {
            labels[SLOT] = slot.name.clone().into();
        }
        if let Some(component) = component {
            labels[COMPONENT] = component.into();
        }
        labels
    }

    fn metadata(&self, name: &str, labels: Value) -> Value {
        json!({"name":name,"labels":labels,"ownerReferences":[{"apiVersion":"v1","kind":"Pod","name":self.pod_name,"uid":self.pod_uid,"controller":true,"blockOwnerDeletion":false}]})
    }

    pub(super) fn policy(&self, slot: &Slot, gateway: bool) -> Value {
        let manager = json!({INSTALLATION:self.common.installation, COMPONENT:"execution-manager"});
        let runner = json!({INSTALLATION:self.common.installation,SLOT:slot.name,COMPONENT:"execution-sandbox"});
        let egress = json!({INSTALLATION:self.common.installation,SLOT:slot.name,COMPONENT:"execution-egress"});
        let (name, spec) = if gateway {
            (
                format!("{}-gateway", slot.name),
                json!({
                    "podSelector":{"matchLabels":self.labels(Some(slot),Some("execution-egress"))},"policyTypes":["Ingress"],
                    "ingress":[{"from":[{"podSelector":{"matchLabels":manager}}],"ports":[{"protocol":"TCP","port":9001}]},
                        {"from":[{"podSelector":{"matchLabels":runner}}],"ports":[{"protocol":"TCP","port":3128}]}]
                }),
            )
        } else {
            (
                slot.name.clone(),
                json!({
                    "podSelector":{"matchLabels":runner},"policyTypes":["Ingress","Egress"],
                    "ingress":[{"from":[{"podSelector":{"matchLabels":manager}}],"ports":[{"protocol":"TCP","port":8080}]}],
                    "egress":[{"to":[{"podSelector":{"matchLabels":egress}}],"ports":[{"protocol":"TCP","port":3128}]}]
                }),
            )
        };
        json!({"apiVersion":"networking.k8s.io/v1","kind":"NetworkPolicy","metadata":self.metadata(&name,self.labels(Some(slot),None)),"spec":spec})
    }

    pub(super) fn pod(&self, slot: &Slot, gateway: bool) -> Value {
        let common = &self.common;
        let mut env = Map::new();
        env.insert(
            "EXECUTION_TIMEOUT_SECONDS".into(),
            (if gateway {
                self.execution_budget()
            } else {
                common.timeout
            })
            .to_string()
            .into(),
        );
        if gateway {
            env.insert("GATEWAY_TOKEN".into(), slot.gateway_token.clone().into());
        } else {
            let proxy = super::endpoint(&slot.gateway_ip, 3128);
            for (key, value) in [
                ("SLOT_TOKEN", slot.runner_token.clone()),
                ("SLOT_MAX_AGE_SECONDS", common.warm_idle_seconds.to_string()),
                ("HOME", "/tmp".into()),
                ("TMPDIR", "/tmp".into()),
                ("RUST_LOG", "warn".into()),
                ("HTTP_PROXY", proxy.clone()),
                ("HTTPS_PROXY", proxy.clone()),
                ("http_proxy", proxy.clone()),
                ("https_proxy", proxy),
                ("NO_PROXY", String::new()),
                ("no_proxy", String::new()),
                ("BACKEND_PUB", common.backend_pub.clone()),
                ("BACKEND_KID", self.key_id.clone()),
                ("API_URL", format!("{}/api/v1", common.callback_url)),
                ("EXECUTOR_REQUIRE_DISPATCH_BINDING", "true".into()),
                (
                    "SLOT_DENIED_ENDPOINT",
                    json!([self.kubernetes_host, self.kubernetes_port]).to_string(),
                ),
                (
                    "SLOT_GATEWAY_ENDPOINT",
                    json!([slot.gateway_ip, 3128]).to_string(),
                ),
            ] {
                env.insert(key.into(), value.into());
            }
        }
        let mut env: Vec<Value> = env
            .into_iter()
            .map(|(key, value)| json!({"name":key,"value":value}))
            .collect();
        env.push(json!({"name":"POD_IP","valueFrom":{"fieldRef":{"fieldPath":"status.podIP"}}}));
        if !gateway {
            env.push(json!({"name":"SLOT_NODE_IP","valueFrom":{"fieldRef":{"fieldPath":"status.hostIP"}}}));
        }
        let uid = if gateway { 65532 } else { 1000 };
        let lifetime = common.warm_idle_seconds + self.execution_budget() + 240;
        let mut spec = json!({
            "automountServiceAccountToken":false,"enableServiceLinks":false,"restartPolicy":"Never","terminationGracePeriodSeconds":5,
            "activeDeadlineSeconds":lifetime,"nodeSelector":self.node_selector,"tolerations":self.tolerations,"imagePullSecrets":self.pull_secrets,
            "securityContext":{"fsGroup":uid},"volumes":[{"name":"tmp","emptyDir":{"medium":"Memory","sizeLimit":format!("{}Mi",self.tmp_mb)}}],
            "containers":[{
                "name":if gateway {"gateway"} else {"runner"},"image":if gateway {&self.gateway_image} else {&self.image},"imagePullPolicy":"IfNotPresent",
                "command":if gateway {vec!["/app/execution-gateway","--tcp"]} else {vec!["/app/execution-slot"]},"env":env,
                "securityContext":{"runAsNonRoot":true,"runAsUser":uid,"runAsGroup":uid,"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,
                    "capabilities":{"drop":["ALL"]},"seccompProfile":{"type":"RuntimeDefault"}},
                "resources":{"requests":{"cpu":if gateway {"50m"} else {"100m"},"memory":if gateway {"32Mi".to_owned()} else {format!("{}Mi",self.memory_mb)}},
                    "limits":{"cpu":if gateway {"500m".to_owned()} else {self.cpus.to_string()},"memory":if gateway {"128Mi".to_owned()} else {format!("{}Mi",self.memory_mb)}}},
                "volumeMounts":[{"name":"tmp","mountPath":"/tmp"}],
                "readinessProbe":{"httpGet":{"path":if gateway {"/health"} else {"/ready"},"port":if gateway {9001} else {8080}},"periodSeconds":1,"timeoutSeconds":1,"failureThreshold":3}
            }]
        });
        if !gateway {
            spec["runtimeClassName"] = self.runtime_class.clone().into();
            spec["dnsPolicy"] = "None".into();
            spec["dnsConfig"] = json!({"nameservers":["127.0.0.1"]});
        }
        let name = format!(
            "{}-{}",
            slot.name,
            if gateway { "gateway" } else { "runner" }
        );
        let mut metadata = self.metadata(
            &name,
            self.labels(
                Some(slot),
                Some(if gateway {
                    "execution-egress"
                } else {
                    "execution-sandbox"
                }),
            ),
        );
        metadata["annotations"] =
            json!({DEADLINE:(slot.born + lifetime as f64).floor().to_string()});
        json!({"apiVersion":"v1","kind":"Pod","metadata":metadata,"spec":spec})
    }

    pub(super) fn bind(&self, run_id: &str, deadline: f64, started: f64) -> Value {
        json!({"metadata":{"labels":{RUN:super::run_hash(run_id)},"annotations":{DEADLINE:deadline.floor().to_string(),"flow-like.io/run-id":run_id}},
            "spec":{"activeDeadlineSeconds":((deadline-started).ceil().max(1.0) as u64)}})
    }

    pub(super) fn cancellation(&self, run_id: &str) -> Value {
        json!({"apiVersion":"v1","kind":"ConfigMap","metadata":{
            "name":self.marker_name(run_id),"labels":{INSTALLATION:self.common.installation,COMPONENT:"execution-cancellation"},
            "annotations":{DEADLINE:(now() + 86400.0 + self.common.budget() as f64 + 240.0).floor().to_string()}
        }})
    }
}

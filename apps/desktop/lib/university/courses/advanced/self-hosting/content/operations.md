Go-live review is Thursday. Priya taped three questions to your monitor: "What happens when you rotate the signing key?" — "Where are last night's backups?" — "Who gets paged when an alert fires?" You can answer two of them from memory. The third one is a trap, and this lesson defuses it.

## 1 · Upgrades without surprises

On the Compose box: `git pull`, then read before you rebuild — the diff of `.env.example`, the hub configuration template, and the release notes. Then:

```bash
docker compose config --quiet
docker compose up -d --build --remove-orphans
```

Schema changes flow through the normal `db-init` dependency; never edit the database schema by hand. On the cluster, `helm upgrade --install` reconciles the release and re-runs the migration Job — the one that executes `prisma db push --accept-data-loss`. Decide once, in writing, whether that runs automatically on upgrade or your DBA runs an approved process with `database.migration.enabled: false`.

## 2 · Question two: backups

Know what's actually yours. `docker compose down` stops containers and keeps named volumes. `docker compose down -v` deletes the Compose-managed PostgreSQL, Redis, Prometheus, Grafana, and Tempo volumes — so back up the database and any monitoring history first. What `-v` never touches: your external object storage. That's the good news and the bad news — deleting the deployment won't destroy your buckets, but nothing in the stack backs them up either. Buckets and the external production database live on your cloud provider's backup schedule, not Compose's.

## 3 · Question one: keys and secrets

Rotating the backend signing key is a three-service event. Re-run `gen-execution-keys.sh --export`, replace all three `BACKEND_*` values, then recreate the API, runtime, and compiler together. Every token signed by the old key is invalid the moment the new one loads — plan the restart window, don't discover it.

The rest is hygiene you already met: `existingSecret` on the cluster, no `docker compose config` output in tickets, a real Grafana admin password, high-entropy `SINK_SECRET` and sink tokens.

One more secret path is easy to miss — the one your builders use inside their apps:

@RuntimeVariables

That's the pilot app's Runtime Variables page in the desktop app: `CRM_API_TOKEN` badged **Secret** and masked, `SUPPORT_API_URL` badged **Runtime** with a plain URL, and the security notice at the bottom spelling out the contract — runtime variables are stored locally on the device and never uploaded; for remote execution, only non-secret values are sent. Read that as an operator: a Secret on a developer's laptop will never follow a flow into your cluster. Credentials for production runs get provisioned server-side, where the run actually lives.

## 4 · Question three: the trap

Start the Compose monitoring profile and you get Prometheus on 9091, Grafana on 3002 (change the admin password before anyone else can reach it), Tempo, and database exporters. Alert rules load and evaluate automatically — API, runtime, Redis, PostgreSQL.

And then: nothing. The bundled Prometheus configuration has an empty Alertmanager list. Alerts fire in the Prometheus UI and page nobody. The Kubernetes chart springs the same trap twice — `monitoring.alertmanager.enabled` only adds a scrape target without deploying an Alertmanager, and the `prometheusRule` values render no resource at all. The honest answer to Priya's third question is "nobody, until we deploy an Alertmanager and validate a test route." Say it before she asks. And don't let a populated dashboard reassure you — a panel proves provisioning, not that every queried metric exists.

## 5 · Day-2 governance

Your platform now hosts other people's decisions too:

@AppVisibilitySettings

That's an app's Visibility Status panel: currently Prototype ("development phase, invite collaborators"), with transitions to Private, Public Request, and Public — plus two footnotes: offline apps can't change visibility, and public transitions require central review. On your deployment, those publication requests land on infrastructure you operate. The App Governance course covers the roles and reviews; your job is keeping the backend behind them healthy.

**Watch out:** an alert rule in a file is not an on-call rotation. Test the route, not the rule.

**Recap**

- Upgrades: read the template diffs, rebuild with `--remove-orphans`, and own the migration Job's data-loss policy.
- `down -v` deletes local volumes but never touches buckets — external storage and databases need their own backup schedule.
- Key rotation restarts three services and kills old tokens; alerts page nobody until an Alertmanager exists.

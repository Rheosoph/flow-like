{{/* vim: set filetype=mustache: */}}

{{/*
Expand the name of the chart.
*/}}
{{- define "flow-like.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "flow-like.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "flow-like.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "flow-like.labels" -}}
helm.sh/chart: {{ include "flow-like.chart" . }}
{{ include "flow-like.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "flow-like.selectorLabels" -}}
app.kubernetes.io/name: {{ include "flow-like.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
API Service labels
*/}}
{{- define "flow-like.api.labels" -}}
{{ include "flow-like.labels" . }}
app.kubernetes.io/component: api
{{- end }}

{{/*
API Service selector labels
*/}}
{{- define "flow-like.api.selectorLabels" -}}
{{ include "flow-like.selectorLabels" . }}
app.kubernetes.io/component: api
{{- end }}

{{/*
Web Service labels
*/}}
{{- define "flow-like.web.labels" -}}
{{ include "flow-like.labels" . }}
app.kubernetes.io/component: web
{{- end }}

{{/*
Web Service selector labels
*/}}
{{- define "flow-like.web.selectorLabels" -}}
{{ include "flow-like.selectorLabels" . }}
app.kubernetes.io/component: web
{{- end }}

{{/*
Executor labels
*/}}
{{- define "flow-like.executor.labels" -}}
{{ include "flow-like.labels" . }}
app.kubernetes.io/component: executor
{{- end }}

{{/*
Compiler labels
*/}}
{{- define "flow-like.compiler.labels" -}}
{{ include "flow-like.labels" . }}
app.kubernetes.io/component: compiler
{{- end }}

{{/*
Signaling labels
*/}}
{{- define "flow-like.signaling.labels" -}}
{{ include "flow-like.labels" . }}
app.kubernetes.io/component: signaling
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "flow-like.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "flow-like.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Backend JWT secret name
*/}}
{{- define "flow-like.jwtSecretName" -}}
{{- if (ne (default "" .Values.jwt.existingSecret) "") -}}
{{- .Values.jwt.existingSecret -}}
{{- else -}}
{{- printf "%s-api-keys" (include "flow-like.fullname" .) -}}
{{- end -}}
{{- end }}

{{/*
Redis secret name. An existing secret is used only when authentication is
enabled; otherwise the chart-managed secret carries the unauthenticated URL.
*/}}
{{- define "flow-like.redisSecretName" -}}
{{- if and .Values.redis.auth.enabled (ne (default "" .Values.redis.auth.existingSecret) "") -}}
{{- .Values.redis.auth.existingSecret -}}
{{- else -}}
{{- printf "%s-redis" (include "flow-like.fullname" .) -}}
{{- end -}}
{{- end }}

{{/*
Database mode: internal (CockroachDB) or external
*/}}
{{- define "flow-like.databaseMode" -}}
{{- $t := (default "internal" .Values.database.type | lower) -}}
{{- if or (eq $t "internal") (eq $t "external") -}}
{{- $t -}}
{{- else -}}
internal
{{- end -}}
{{- end }}

{{/*
Database secret name
*/}}
{{- define "flow-like.databaseSecretName" -}}
{{- if eq (include "flow-like.databaseMode" .) "internal" -}}
{{- printf "%s-cockroachdb" (include "flow-like.fullname" .) -}}
{{- else -}}
{{- if (ne (default "" .Values.database.external.existingSecret) "") -}}
{{- .Values.database.external.existingSecret -}}
{{- else -}}
{{- printf "%s-db-external" (include "flow-like.fullname" .) -}}
{{- end -}}
{{- end -}}
{{- end }}

{{/*
Database host
*/}}
{{- define "flow-like.databaseHost" -}}
{{- if eq (include "flow-like.databaseMode" .) "internal" -}}
{{- printf "%s-cockroachdb-public" (include "flow-like.fullname" .) -}}
{{- else -}}
{{- .Values.database.external.host -}}
{{- end -}}
{{- end }}

{{/*
Whether to use CockroachDB native schema (no postgres mirror needed)
*/}}
{{- define "flow-like.useCockroachSchema" -}}
{{- if eq (include "flow-like.databaseMode" .) "internal" -}}
true
{{- else -}}
false
{{- end -}}
{{- end }}

{{/*
Get the Redis URL
*/}}
{{- define "flow-like.redisUrl" -}}
{{- if .Values.redis.auth.enabled }}
{{- printf "redis://:%s@%s-redis-master:6379" .Values.redis.auth.password (include "flow-like.fullname" .) }}
{{- else }}
{{- printf "redis://%s-redis-master:6379" (include "flow-like.fullname" .) }}
{{- end }}
{{- end }}

{{/*
Storage secret name - based on provider
*/}}
{{- define "flow-like.storageSecretName" -}}
{{- $provider := default "s3" .Values.storage.provider -}}
{{- if eq $provider "aws" -}}
  {{- if (ne (default "" .Values.storage.aws.existingSecret) "") -}}
    {{- .Values.storage.aws.existingSecret -}}
  {{- else -}}
    {{- printf "%s-storage" (include "flow-like.fullname" .) -}}
  {{- end -}}
{{- else if eq $provider "azure" -}}
  {{- if (ne (default "" .Values.storage.azure.existingSecret) "") -}}
    {{- .Values.storage.azure.existingSecret -}}
  {{- else -}}
    {{- printf "%s-storage" (include "flow-like.fullname" .) -}}
  {{- end -}}
{{- else if eq $provider "gcp" -}}
  {{- if (ne (default "" .Values.storage.gcp.existingSecret) "") -}}
    {{- .Values.storage.gcp.existingSecret -}}
  {{- else -}}
    {{- printf "%s-storage" (include "flow-like.fullname" .) -}}
  {{- end -}}
{{- else if eq $provider "r2" -}}
  {{- if (ne (default "" .Values.storage.r2.existingSecret) "") -}}
    {{- .Values.storage.r2.existingSecret -}}
  {{- else -}}
    {{- printf "%s-storage" (include "flow-like.fullname" .) -}}
  {{- end -}}
{{- else -}}
  {{- if (ne (default "" .Values.storage.s3.existingSecret) "") -}}
    {{- .Values.storage.s3.existingSecret -}}
  {{- else -}}
    {{- printf "%s-storage" (include "flow-like.fullname" .) -}}
  {{- end -}}
{{- end -}}
{{- end }}

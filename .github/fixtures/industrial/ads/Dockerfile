FROM mcr.microsoft.com/dotnet/sdk:8.0.414-bookworm-slim@sha256:ff8311847c54c04d1a14c488362807997d59b61372da5095a95f89cbcda7f9b7 AS build
RUN git init /src && git -C /src remote add origin https://github.com/Beckhoff/TF6000_ADS_DOTNET_V5_Samples.git \
    && git -C /src fetch --depth 1 origin 89e2de2d2df82514b91ca8d84705810cebda3515 \
    && git -C /src checkout --detach FETCH_HEAD
WORKDIR /src/Sources/DockerSamples
# Pin the vendor SDK version instead of the sample's floating 6.2.* references.
RUN sed -i 's/6\.2\.\*/6.2.521/g' AdsRouterConsole/AdsRouterConsole.csproj AdsServer/AdsServer.csproj \
    && dotnet publish AdsRouterConsole/AdsRouterConsole.csproj -c Release -o /out/router /p:UseAppHost=false \
    && dotnet publish AdsServer/AdsServer.csproj -c Release -o /out/server /p:UseAppHost=false

FROM mcr.microsoft.com/dotnet/runtime:8.0.20-bookworm-slim@sha256:e9aabde56bb3e55d416d4e926032af75fed831da70bb428d556120ee2649f8b0
RUN apt-get update && apt-get install -y --no-install-recommends python3 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /out /app
COPY --from=build /src/LICENSE.md /usr/share/licenses/beckhoff-samples/LICENSE.md
COPY start.py healthcheck.py /fixture/
ENV AmsRouter__Name=FlowLikeFixture \
    AmsRouter__NetId=42.42.42.42.1.1 \
    AmsRouter__ChannelProtocol=Ads \
    AmsRouter__LoopbackIP=127.0.0.1 \
    AmsRouter__LoopbackPort=48900 \
    AmsRouter__RemoteConnections__0__Name=FlowLikeTests \
    AmsRouter__RemoteConnections__0__NetId=10.11.12.13.1.1 \
    AmsRouter__RemoteConnections__1__Name=Healthcheck \
    AmsRouter__RemoteConnections__1__Address=127.0.0.1 \
    AmsRouter__RemoteConnections__1__NetId=1.2.3.4.1.1 \
    Logging__LogLevel__Default=Warning
USER 65534:65534
EXPOSE 48898
HEALTHCHECK --interval=3s --timeout=3s --start-period=15s --retries=30 CMD ["python3", "/fixture/healthcheck.py"]
ENTRYPOINT ["python3", "/fixture/start.py"]

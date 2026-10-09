FROM python:3.13-slim@sha256:bf44cdfcb76cd3b41e879bc058fc37ec5872002ccfde7fcb765e218cde0cd79c AS build
RUN apt-get update && apt-get install -y --no-install-recommends gcc libc6-dev git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN git init /src && git -C /src remote add origin https://github.com/libplctag/libplctag.git \
    && git -C /src fetch --depth 1 origin 7712ed7f1ee05ba0ebfca641b3f5c43da52a66ed \
    && git -C /src checkout --detach FETCH_HEAD \
    && gcc -O2 -pthread -I/src/src /src/src/tools/ab_server/*.c -lm -o /ab_server

FROM python:3.13-slim@sha256:bf44cdfcb76cd3b41e879bc058fc37ec5872002ccfde7fcb765e218cde0cd79c
COPY --from=build /ab_server /usr/local/bin/ab_server
COPY --from=build /src/LICENSE-1.MPL /src/LICENSE-2.LGPL /usr/share/licenses/libplctag/
COPY healthcheck.py /fixture/healthcheck.py
USER 65534:65534
EXPOSE 44818
HEALTHCHECK --interval=3s --timeout=3s --start-period=5s --retries=20 CMD ["python", "/fixture/healthcheck.py"]
ENTRYPOINT ["ab_server", "--plc=ControlLogix", "--path=1,0"]
CMD ["--tag=Bool:BOOL[4]", "--tag=Sint:SINT[4]", "--tag=Int:INT[4]", "--tag=Dint:DINT[4]", "--tag=Lint:LINT[4]", "--tag=Real:REAL[4]", "--tag=Lreal:LREAL[4]", "--tag=Text:STRING[4]"]

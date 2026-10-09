FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates git g++ make libssl-dev libjsoncpp-dev python3 && rm -rf /var/lib/apt/lists/*
RUN git init /vendor && cd /vendor && git remote add origin https://github.com/FieldCommGroup/hipserver.git && git fetch --depth 1 origin 8c5b565a93c3bb50a8eec47df62d2b3ea2edd45c && git checkout --detach FETCH_HEAD
WORKDIR /vendor/hipserver/Server
# Containers have no Raspberry Pi reset/write-protect GPIO, including on ARM hosts.
# Select the vendor's existing no-GPIO branches without changing its protocol code.
RUN sed -i 's/#if !defined(__x86_64__)/#if 0/' main.c factory_reset.c hsreadonlycommandsmanager.cpp
RUN make -j2
COPY device.cpp /fixture/device.cpp
# The vendor server and PDU codec handle the wire protocol. The companion only
# supplies deterministic device values, using the vendor's application interface.
RUN g++ -std=c++11 -DLINUX -ffunction-sections -fdata-sections -I. -I../Common -I../Realtime -I../AppConnect -I../safestringlib/include /fixture/device.cpp ../AppConnect/app.cpp ../AppConnect/apppdu.cpp ../AppConnect/tppdu.cpp ../Realtime/toolqueues.cpp ../Realtime/toolutils.c ../Realtime/toolsems.c ../Realtime/toolthreads.c ../Realtime/toolsigs.c -Wl,--gc-sections -pthread -lrt ../safestringlib/libsafestring.a -o /fixture/device
COPY ready.py /fixture/ready.py
HEALTHCHECK --interval=3s --timeout=5s --start-period=10s --retries=10 CMD ["python3", "/fixture/ready.py"]
EXPOSE 5094
CMD ["./hipserver", "-p", "5094", "/fixture/device"]

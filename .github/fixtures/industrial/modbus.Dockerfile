FROM python:3.13-slim@sha256:bf44cdfcb76cd3b41e879bc058fc37ec5872002ccfde7fcb765e218cde0cd79c

WORKDIR /fixture
COPY modbus-requirements.txt .
RUN pip install --no-cache-dir --requirement modbus-requirements.txt
COPY modbus_server.py .

USER 65534:65534
EXPOSE 5020
HEALTHCHECK --interval=2s --timeout=3s --retries=15 CMD ["python", "modbus_server.py", "--healthcheck"]
CMD ["python", "-u", "modbus_server.py"]

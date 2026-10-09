FROM python:3.13-slim@sha256:bf44cdfcb76cd3b41e879bc058fc37ec5872002ccfde7fcb765e218cde0cd79c

WORKDIR /fixture
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY peer.py .
USER 65534:65534
EXPOSE 8080
HEALTHCHECK --interval=2s --timeout=5s --retries=30 CMD ["python", "peer.py", "--healthcheck"]
CMD ["python", "-u", "peer.py"]

FROM python:3.13.15-slim-bookworm@sha256:ed86c82274b3c69b52fb5820f358f0bd7df0b603332063cb5c6e32bd220c3e6e
WORKDIR /opt/object-store
COPY requirements.txt ./
RUN pip install --no-cache-dir --requirement requirements.txt
COPY bootstrap.py conformance.py ./
USER 65532:65532
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1
ENTRYPOINT ["python", "/opt/object-store/bootstrap.py"]

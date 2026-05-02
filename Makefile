# Docker: usar estos objetivos evita el estado Compose inconsistente que provoca
# «No such container» al usar solo `docker compose up` tras borrar contenedores a mano.
.PHONY: docker-down docker-up docker-rebuild docker-logs docker-smoke

docker-down:
	docker compose down --remove-orphans

docker-up:
	@bash scripts/docker-stack-up.sh

docker-rebuild:
	@bash scripts/docker-stack-up.sh --build

docker-logs:
	docker compose logs -f futurefin-api

docker-smoke:
	@curl -sf http://127.0.0.1:8080/v1/health && echo

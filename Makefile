.DEFAULT_GOAL := help

.PHONY: help
help: ## List targets in this Makefile
	@awk '\
		BEGIN { FS = ":$$|:[^#]+|:.*?## "; OFS="\t" }; \
		/^[0-9a-zA-Z_-]+?:/ { print $$1, $$2 } \
	' $(MAKEFILE_LIST) \
		| sort --dictionary-order \
		| column --separator $$'\t' --table --table-wrap 2 --output-separator '    '

services: ## Start development services (db, etc.)
	docker-compose up -d

services-stop: ## Stop development services
	docker-compose stop

services-down: ## _Destroy_ development services (this deletes data!)
	docker-compose down

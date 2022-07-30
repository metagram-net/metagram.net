.DEFAULT_GOAL := help

.PHONY: help
help: ## List targets in this Makefile
	@awk '\
		BEGIN { FS = ":$$|:[^#]+|:.*?## "; OFS="\t" }; \
		/^[0-9a-zA-Z_-]+?:/ { print $$1, $$2 } \
	' $(MAKEFILE_LIST) \
		| sort --dictionary-order \
		| column --separator $$'\t' --table --table-wrap 2 --output-separator '    '

.PHONY: services
services: ## Start development services (db, etc.)
	docker-compose up -d

.PHONY: services-stop
services-stop: ## Stop development services
	docker-compose stop

.PHONY: services-down
services-down: ## _Destroy_ development services (this deletes data!)
	docker-compose down

.PHONY: migrate
migrate: ## Run local database migrations
	diesel migration run

.PHONY: css
css: ## Build the CSS
	npx tailwindcss -i ./css/index.css -o ./dist/index.css

.PHONY: css-watch
css-watch: ## Run the CSS build in watch mode
	npx tailwindcss -i ./css/index.css -o ./dist/index.css --watch

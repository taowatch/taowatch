# syntax=docker/dockerfile:1
FROM node:26.13.1-trixie AS build
WORKDIR /app
COPY package.json package-lock.json* ./
RUN npm ci || npm install
COPY . .
RUN npm run build

FROM caddy:2.11.9
COPY Caddyfile /etc/caddy/Caddyfile
COPY --from=build /app/dist /srv

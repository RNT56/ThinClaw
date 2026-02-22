FROM node:22-alpine

# Install basic tools for healthchecks
RUN apk add --no-cache curl

WORKDIR /app

# Copy package files first
COPY package.json ./

# Install dependencies
RUN npm install

# Copy source code
COPY main.js ./

# Create data directory
RUN mkdir -p /app/state
ENV OPENCLAW_HOME=/app/state
ENV PORT=18789

# Expose port
EXPOSE 18789

# Start command
CMD ["node", "main.js"]

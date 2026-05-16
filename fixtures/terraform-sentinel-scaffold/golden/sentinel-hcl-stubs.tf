# Golden reference: Expected stubs when sentinel.hcl defines enforcement levels
# agentcarousel golden file — do not edit without bumping bundle_version
# Last updated: 2026-04-20 | bundle: agentcarousel/terraform-sentinel-scaffold@1.0.0

# ─────────────────────────────────────────────────────────────────────────────
# MANDATORY — require-tags.sentinel
# https://registry.terraform.io/providers/hashicorp/aws/latest/docs/resources/instance
resource "aws_instance" "example" {
  ami           = "ami-00000000"   # placeholder — update for your region
  instance_type = "t3.micro"

  tags = {
    # Sentinel policy: tags must not be null
    # Required tags for your organization (update as needed):
    Name        = "example-instance"
    Environment = ""
    Owner       = ""
  }
}

# ADVISORY — check-naming.sentinel
# https://registry.terraform.io/providers/hashicorp/aws/latest/docs/resources/s3_bucket
resource "aws_s3_bucket" "example" {
  # Sentinel policy: bucket name must match "^prod-"
  bucket = "prod-example-bucket"
}

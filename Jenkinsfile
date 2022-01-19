pipeline {
    agent any
    environment {
        ecr_repo_domain = '192549843005.dkr.ecr.eu-west-1.amazonaws.com'
    }
    stages {
        stage('ecr-login') {
            steps {
                ecrLogin(env.ecr_repo_domain, 'eu-west-1')
            }
        }
        stage('build') {
            environment {
                image_repo = "${ecr_repo_domain}/concordium/transaction-logger"
                image_name = "${image_repo}:${image_tag}"
            }
            steps {
                sh '''\
                    docker build \
                      --build-arg build_image="${build_image}" \
                      --label build_image="${build_image}" \
                      --build-arg base_image="${base_image}" \
                      --label base_image="${base_image}" \
                      -f Dockerfile \
                      -t "${image_name}" \
                      .
                '''.stripIndent()
            }
        }
        stage('push') {
            steps {
                sh 'docker push "${image_name}"'
            }
        }
    }
}
